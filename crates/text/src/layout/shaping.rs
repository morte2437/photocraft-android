//! Per-paragraph preparation of a text layer's layout: the paragraph's shaped text (prefix, case
//! mapping, tabs), the parley shaping pass over it, and its line breaking.

use std::borrow::Cow;
use std::cell::Cell;
use std::ops::Range;

use parley::{Alignment, AlignmentOptions, FontFeatures, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use photocraft_doc::text::{CharStyle, ParagraphStyle, TextAlign, TextDirection};

use super::area::{LineMetrics, MAX_AREA_PASSES, area_metrics, break_area};
use super::paragraphs::{TabWidth, strip_break, tab_widths};
use super::style::{
    SYNTHETIC_SMALL_CAPS_SCALE, SmallCapsMode, drawn_style, effective_horizontal_scale, feature_list, hangul_runs, probe_smcp, small_caps_mode, style_props,
};
use super::vertical::vertical_class;
use super::{Env, FORCED_LINE_BREAK, Flow, LRM, RLM, RunBrush, Slot, VClass};
use crate::fonts::FontDb;
use crate::layout::DEFAULT_TAB_STOP_PT;

/// The character styles of `runs`, with families resolved (PostScript names from PSDs, unknown
/// families) and drawn the way they are laid out.
pub(super) fn resolve_run_styles(fonts: &mut FontDb, runs: &[photocraft_doc::text::TextRun]) -> Vec<CharStyle> {
    runs.iter()
        .map(|r| {
            let mut s = r.style.clone();
            adopt_postscript_face(fonts, &mut s);
            // Drawn with a fallback for now; a host that serves this family fetches it (`served`).
            if !fonts.has_family(&s.font_family) {
                crate::served::request(&s.font_family);
            }
            fonts.select_named_face(&mut s);
            drawn_style(&mut s);
            s
        })
        .collect()
}

/// A style naming a PostScript face the family doesn't carry takes the family the face resolves to.
fn adopt_postscript_face(fonts: &mut FontDb, s: &mut CharStyle) {
    let Some(ps) = s.postscript_name.clone() else {
        return;
    };
    if fonts.faces(&s.font_family).iter().any(|f| f.postscript_name.as_deref() == Some(&ps)) {
        return;
    }
    let f = fonts.resolve_postscript(&ps);
    // An exact face match wins; a guessed family only fills a missing family.
    if f.exact || (!fonts.has_family(&s.font_family) && fonts.has_family(&f.family)) {
        s.font_family = f.family;
        s.weight = f.weight;
        s.italic = f.italic;
    }
}

/// Start offset of each item, given the items' lengths.
pub(super) fn running_starts(lens: impl Iterator<Item = usize>) -> Vec<usize> {
    lens.scan(0, |a, len| {
        let s = *a;
        *a += len;
        Some(s)
    })
    .collect()
}

/// How each style sets small caps. Fixed for the whole layout, and the font probe only matters for
/// styles that use small caps. A real `smcp` substitution must receive lowercase source text; only
/// the fallback path uppercases it and scales it down.
pub(super) fn small_caps_modes(fonts: &mut FontDb, styles: &[CharStyle]) -> Vec<SmallCapsMode> {
    styles
        .iter()
        .map(|style| match style.caps {
            photocraft_doc::text::Caps::SmallCaps => small_caps_mode(style.caps, probe_smcp(fonts, style)),
            _ => SmallCapsMode::None,
        })
        .collect()
}

/// The text handed to the shaper for one paragraph.
struct ParaText {
    ptext: String,
    /// Ranges in `ptext` for lowercase characters rendered as synthetic small caps. They have the
    /// same UTF-8 length as their uppercase form, so the layer's byte-based run and caret offsets
    /// remain unchanged.
    synthetic_small_caps: Vec<(Range<usize>, f32)>,
    /// Offsets in `ptext` of the spaces standing in for tabs.
    tabs: Vec<usize>,
}

/// The shaped text of the paragraph at `range` (`content_end` excludes its break), led by `prefix`.
fn paragraph_text(env: &Env<'_>, prefix: &str, range: &Range<usize>, content_end: usize) -> ParaText {
    let content = &env.text[range.start..content_end];
    let mut ptext = String::with_capacity(prefix.len() + content.len());
    ptext.push_str(prefix);
    let mut synthetic_small_caps = Vec::new();
    let mut tabs = Vec::new();
    for (i, ch) in content.char_indices() {
        // A forced line break ends the line but not the paragraph. The line breaker knows
        // it as a newline, which has the same length, so text offsets don't move.
        if ch == FORCED_LINE_BREAK {
            ptext.push('\n');
            continue;
        }
        // Imported PSD text can contain literal tab controls. Font shaping may
        // render those as .notdef boxes; a space preserves the one-byte source
        // and style/caret offsets while supplying a real whitespace advance. The space
        // is widened to its tab stop after line breaking (`tab_px`).
        if ch == '\t' {
            tabs.push(ptext.len());
            ptext.push(' ');
            continue;
        }
        let si = env.style_at(range.start + i);
        let synthetic = env.small_caps[si] == SmallCapsMode::Synthetic;
        push_cased(&mut ptext, &mut synthetic_small_caps, ch, &env.styles[si], synthetic, env.k);
    }
    ParaText { ptext, synthetic_small_caps, tabs }
}

/// Appends `ch` as its style sets it: uppercased for all caps and synthetic small caps (when the
/// uppercase form has the same length, so offsets don't move), recording small-cap pieces.
fn push_cased(ptext: &mut String, small_caps: &mut Vec<(Range<usize>, f32)>, ch: char, style: &CharStyle, synthetic: bool, k: f32) {
    let caps = style.caps;
    if caps == photocraft_doc::text::Caps::AllCaps || synthetic {
        let up: String = ch.to_uppercase().collect();
        if up.len() == ch.len_utf8() {
            ptext.push_str(&up);
            if caps == photocraft_doc::text::Caps::SmallCaps && ch.is_lowercase() {
                let end = ptext.len();
                small_caps.push((end - up.len()..end, style.size_pt * k * SYNTHETIC_SMALL_CAPS_SCALE));
            }
            return;
        }
    }
    ptext.push(ch);
}

/// One run piece of vertical type, whose upright stretches get the vertical alternates and their
/// class in the brush.
struct VerticalPiece<'a> {
    style: &'a CharStyle,
    run: usize,
    small_caps: SmallCapsMode,
    /// Offset of the piece in the paragraph text.
    start: usize,
}

impl VerticalPiece<'_> {
    fn push_upright(&self, b: &mut parley::RangedBuilder<'_, RunBrush>, from: usize, to: usize, cls: VClass) {
        if cls == VClass::Rotate || from >= to {
            return;
        }
        let mut feats = feature_list(self.style, self.small_caps);
        feats.push("\"vert\" 1".into());
        let r = (self.start + from)..(self.start + to);
        b.push(StyleProperty::FontFeatures(FontFeatures::Source(Cow::Owned(feats.join(", ")))), r.clone());
        b.push(StyleProperty::Brush(RunBrush(self.run as u32, cls as u8)), r);
    }

    /// Splits `piece` into stretches of one vertical class and styles each.
    fn push_all(&self, b: &mut parley::RangedBuilder<'_, RunBrush>, piece: &str) {
        let mut seg: Option<(usize, VClass)> = None;
        for (i, ch) in piece.char_indices() {
            let cls = match vertical_class(ch) {
                VClass::Rotate if !self.style.rotate_roman && upright_roman(ch) => VClass::Upright,
                cls => cls,
            };
            match seg {
                Some((_, c)) if c == cls => {}
                Some((from, c)) => {
                    self.push_upright(b, from, i, c);
                    seg = Some((i, cls));
                }
                None => seg = Some((i, cls)),
            }
        }
        if let Some((from, c)) = seg {
            self.push_upright(b, from, piece.len(), c);
        }
    }
}

/// What line placement needs of one paragraph: its text, styles, indents and the extents its lines
/// break at.
pub(super) struct Para<'a> {
    pub(super) env: &'a Env<'a>,
    pub(super) index: usize,
    pub(super) ps: ParagraphStyle,
    pub(super) start: usize,
    pub(super) content_end: usize,
    pub(super) prefix: &'static str,
    ptext: String,
    synthetic_small_caps: Vec<(Range<usize>, f32)>,
    /// Offsets in the paragraph text of the spaces standing in for tabs.
    pub(super) tabs: Vec<usize>,
    /// Runs intersecting the paragraph: `si0..si_end`.
    si0: usize,
    si_end: usize,
    roman: Vec<(Range<usize>, f32, f32, usize)>,
    fallback: Vec<String>,
    pub(super) indent_start: f32,
    indent_end: f32,
    pub(super) first_indent: f32,
    /// Box extent along the lines: where they start.
    pub(super) line_origin: f32,
    /// Box extent along the lines, less the indents; `None` for point text.
    pub(super) avail: Option<f32>,
    pub(super) break_scale: f32,
    pub(super) tab_px: f32,
    /// Line-breaking passes left for this paragraph's area type (see `break_area`).
    area_budget: Cell<usize>,
}

impl<'a> Para<'a> {
    pub(super) fn new(env: &'a Env<'a>, fonts: &FontDb, index: usize, range: Range<usize>, ps: ParagraphStyle) -> Self {
        let k = env.k;
        let content_end = strip_break(env.text, &range);
        let prefix = match ps.direction {
            TextDirection::Auto => "",
            TextDirection::Ltr => LRM,
            TextDirection::Rtl => RLM,
        };
        let ParaText { ptext, synthetic_small_caps, tabs } = paragraph_text(env, prefix, &range, content_end);
        // Scripts the loaded fonts may lack (Arabic, Japanese, …): a host serving a font for them
        // fetches it (`served`); it joins the fallback stack once it arrives.
        crate::served::request_for_text(&ptext);
        let fallback = fonts.fallback_stack().map(str::to_string).collect();
        let si0 = env.style_at(range.start);
        let si_end = env.run_starts.partition_point(|&rs| rs < content_end).max(si0 + 1).min(env.styles.len());
        let indent_start = ps.start_indent_pt * k;
        let indent_end = ps.end_indent_pt * k;
        let first_indent = ps.first_line_indent_pt * k;
        // Box extent along the lines: width, or height for vertical type (columns).
        let (box_x, box_y, box_w, box_h) = env.box_rect;
        let (line_origin, line_len) = if env.vertical { (box_y, box_h) } else { (box_x, box_w) };
        let avail = if env.is_box { Some((line_len - indent_start - indent_end).max(1.0)) } else { None };
        let mut para = Self {
            env,
            index,
            ps,
            start: range.start,
            content_end,
            prefix,
            ptext,
            synthetic_small_caps,
            tabs,
            si0,
            si_end,
            roman: Vec::new(),
            fallback,
            indent_start,
            indent_end,
            first_indent,
            line_origin,
            avail,
            break_scale: 1.0,
            tab_px: DEFAULT_TAB_STOP_PT * k,
            area_budget: Cell::new(MAX_AREA_PASSES),
        };
        para.break_scale = para.horizontal_break_scale();
        para.roman = para.roman_ranges();
        para
    }

    fn roman_ranges(&self) -> Vec<(Range<usize>, f32, f32, usize)> {
        let env = self.env;
        let mut roman = Vec::new();
        if env.vertical {
            for (ri, st) in env.styles.iter().enumerate().take(self.si_end).skip(self.si0) {
                let rs = env.run_starts[ri];
                let (a, z) = (rs.max(self.start), (rs + env.runs[ri].len).min(self.content_end));
                if a >= z || st.rotate_roman {
                    continue;
                }
                let base = a - self.start + self.prefix.len();
                let piece = self.ptext.get(base..base + (z - a)).unwrap_or("");
                let px = st.size_pt * env.k;
                for (i, ch) in piece.char_indices() {
                    if upright_roman(ch) && !ch.is_whitespace() {
                        roman.push(((base + i)..(base + i + ch.len_utf8()), st.tracking / 1000.0 * px, px, ri));
                    }
                }
            }
        }
        roman
    }

    pub(super) fn first_style(&self) -> &'a CharStyle {
        &self.env.styles[self.si0]
    }

    /// Text offset of the paragraph text's byte `o`, as the layer's text offset.
    pub(super) fn map_offset(&self, o: usize) -> usize {
        (self.start + o.saturating_sub(self.prefix.len())).min(self.content_end)
    }

    /// The paragraph text up to the end of the run piece at text range `range`.
    pub(super) fn text_of(&self, range: Range<usize>) -> &str {
        self.ptext.get(range).unwrap_or("")
    }

    /// Horizontal scale is applied after shaping, so paragraph text breaks at the box (or area span)
    /// width divided by it (#5565). Mixed scales break at the largest, so no line overflows the box.
    fn horizontal_break_scale(&self) -> f32 {
        let env = self.env;
        if !env.wraps {
            return 1.0;
        }
        let scale = (self.si0..self.si_end)
            .filter(|&ri| {
                let rs = env.run_starts.get(ri).copied().unwrap_or(0);
                let end = rs.saturating_add(env.runs.get(ri).map_or(0, |r| r.len));
                ri == self.si0 || (rs.max(self.start) < end.min(self.content_end))
            })
            .filter_map(|ri| env.styles.get(ri))
            .map(effective_horizontal_scale)
            .fold(0.0f32, f32::max);
        if scale > 0.0 { scale } else { 1.0 }
    }

    /// Shapes the paragraph, each tab widened by its letter spacing in `tab_spacing` (px).
    fn shape(&self, lcx: &mut LayoutContext<RunBrush>, fcx: &mut parley::FontContext, tab_spacing: &[f32], roman_spacing: &[f32]) -> Layout<RunBrush> {
        let env = self.env;
        let mut b = lcx.ranged_builder(fcx, &self.ptext, 1.0, false);
        // Paragraph-start style as the default (covers the direction mark and empty
        // paragraphs), then every run piece intersecting this paragraph.
        for p in style_props(&env.styles[self.si0], env.k, &self.fallback, self.si0 as u32, env.small_caps[self.si0]) {
            b.push_default(p);
        }
        // Paragraph text breaks a word wider than the box at the box edge, as Photoshop
        // does (#6694): parley's default only breaks at UAX #14 opportunities and lets the
        // word run out of the box. Point text has no width to overflow.
        if env.is_box {
            b.push_default(StyleProperty::OverflowWrap(parley::style::OverflowWrap::BreakWord));
        }
        for ri in self.si0..self.si_end {
            self.push_run_piece(&mut b, ri);
        }
        for (range, size) in &self.synthetic_small_caps {
            b.push(StyleProperty::FontSize(*size), range.clone());
        }
        // Korean breaks lines at spaces, between words, as Photoshop sets it; Unicode's
        // default rules break between any two Hangul syllables, mid-word (#5959).
        for range in hangul_runs(&self.ptext) {
            b.push(StyleProperty::WordBreak(parley::WordBreak::KeepAll), range);
        }
        for (&at, &spacing) in self.tabs.iter().zip(tab_spacing) {
            b.push(StyleProperty::LetterSpacing(spacing), at..at + 1);
        }
        for ((range, _, _, ri), &spacing) in self.roman.iter().zip(roman_spacing) {
            b.push(StyleProperty::LetterSpacing(spacing), range.clone());
            let mut feats = feature_list(&env.styles[*ri], env.small_caps[*ri]);
            feats.extend(["kern", "liga", "clig", "dlig"].map(|f| format!("\"{f}\" 0")));
            b.push(StyleProperty::FontFeatures(FontFeatures::Source(Cow::Owned(feats.join(", ")))), range.clone());
        }
        b.build(&self.ptext)
    }

    /// Styles the part of run `ri` inside this paragraph.
    fn push_run_piece(&self, b: &mut parley::RangedBuilder<'_, RunBrush>, ri: usize) {
        let env = self.env;
        let rs = env.run_starts[ri];
        let (a, z) = (rs.max(self.start), (rs + env.runs[ri].len).min(self.content_end));
        if a >= z {
            return;
        }
        let range = (a - self.start + self.prefix.len())..(z - self.start + self.prefix.len());
        for p in style_props(&env.styles[ri], env.k, &self.fallback, ri as u32, env.small_caps[ri]) {
            b.push(p, range.clone());
        }
        if env.vertical {
            let piece = VerticalPiece { style: &env.styles[ri], run: ri, small_caps: env.small_caps[ri], start: range.start };
            piece.push_all(b, self.text_of(range));
        }
    }

    /// Where an area paragraph's first row goes: no higher than one leading (plus paragraph
    /// spacing) below the previous paragraph's last line.
    fn area_origin(&self, m: LineMetrics, flow: &Flow) -> f32 {
        let space_before = if flow.prev_baseline.is_some() { self.ps.space_before_pt * self.env.k } else { 0.0 };
        match (&self.env.area, flow.prev_baseline) {
            (Some(_), Some(b)) => b + m.leading + flow.pending_space + space_before,
            (Some(a), None) => a.bounds[1] + m.ascent,
            (None, _) => 0.0,
        }
    }

    /// Breaks `layout` into lines; for area type also returns each line's slot and whether the
    /// shape ran full.
    fn break_lines(&self, layout: &mut Layout<RunBrush>, flow: &Flow) -> (Vec<Slot>, bool) {
        let env = self.env;
        if self.first_indent != 0.0 {
            layout.set_text_indent(self.first_indent / self.break_scale, parley::IndentOptions::default());
        }
        let Some(a) = &env.area else {
            layout.break_all_lines(self.avail.map(|a| a / self.break_scale));
            return (Vec::new(), false);
        };
        // Plan every row with the paragraph's real line metrics (its tallest font and
        // largest leading, measured on the unwrapped paragraph), so no wrapped line comes
        // out taller than the row it was planned in and no second plan is needed.
        layout.break_all_lines(None);
        let m = area_metrics(layout, env.styles, &self.ps, env.k, self.first_style(), env.vertical);
        let from = self.area_origin(m, flow);
        let first_px = self.first_style().size_pt * env.k;
        break_area(layout, a, from, m, (self.indent_start, self.indent_end), first_px, self.break_scale, &self.area_budget)
    }

    /// The widths of the tabs on the broken lines of `layout`.
    fn tab_widths(&self, layout: &Layout<RunBrush>) -> Vec<TabWidth> {
        tab_widths(layout, &self.tabs, self.env.styles, self.indent_start, self.first_indent, self.tab_px)
    }

    /// Shapes and breaks the paragraph into lines (and area slots).
    ///
    /// A tab is never wider than a space plus one tab interval, so breaking lines at that width
    /// keeps the expanded tabs inside the box, but can wrap early. Break again with the tab widths
    /// that layout gives until they agree; if they don't settle, keep the safe layout.
    pub(super) fn break_paragraph(&self, lcx: &mut LayoutContext<RunBrush>, fcx: &mut parley::FontContext, flow: &Flow) -> (Layout<RunBrush>, Vec<Slot>, bool) {
        let wide = if self.env.wraps { self.tab_px } else { 0.0 };
        let mut roman_spacing: Vec<f32> = self.roman.iter().map(|r| r.1).collect();
        let mut layout = self.shape(lcx, fcx, &vec![wide; self.tabs.len()], &roman_spacing);
        if !self.roman.is_empty() {
            let _ = self.break_lines(&mut layout, flow);
            let advances = cluster_advances(&layout, &self.roman);
            roman_spacing = self.roman.iter().zip(&advances).map(|((_, track, em, _), a)| a.map_or(*track, |a| em + track - (a - track))).collect();
            layout = self.shape(lcx, fcx, &vec![wide; self.tabs.len()], &roman_spacing);
        }
        let (mut slots, mut full) = self.break_lines(&mut layout, flow);
        if self.env.wraps
            && !self.tabs.is_empty()
            && let Some((settled, settled_slots, settled_full)) = self.settle_tabs(lcx, fcx, &layout, flow, &roman_spacing)
        {
            layout = settled;
            slots = settled_slots;
            full = settled_full;
        }
        (layout, slots, full)
    }

    /// Re-breaks until the tab widths the layout gives agree with the ones it was shaped for.
    fn settle_tabs(
        &self,
        lcx: &mut LayoutContext<RunBrush>,
        fcx: &mut parley::FontContext,
        layout: &Layout<RunBrush>,
        flow: &Flow,
        roman_spacing: &[f32],
    ) -> Option<(Layout<RunBrush>, Vec<Slot>, bool)> {
        let first = self.tab_widths(layout);
        let mut want: Vec<f32> = first.iter().map(|t| t.width).collect();
        for _ in 0..4 {
            let spacing: Vec<f32> = first.iter().zip(&want).map(|(t, w)| w / t.scale - (t.advance - self.tab_px)).collect();
            let mut next = self.shape(lcx, fcx, &spacing, roman_spacing);
            let (next_slots, next_full) = self.break_lines(&mut next, flow);
            let got: Vec<f32> = self.tab_widths(&next).iter().map(|t| t.width).collect();
            if got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 0.01) {
                return Some((next, next_slots, next_full));
            }
            want = got;
        }
        None
    }

    /// How parley aligns the paragraph's lines.
    pub(super) fn alignment(&self) -> Alignment {
        if !self.env.wraps {
            return Alignment::Left;
        }
        match self.ps.align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Center => Alignment::Center,
            TextAlign::Right => Alignment::Right,
            _ => Alignment::Justify,
        }
    }

    pub(super) fn align_options(&self) -> AlignmentOptions {
        AlignmentOptions { align_when_overflowing: !self.env.wraps }
    }
}

/// Whether vertical type sets `c` upright when its style doesn't rotate roman text (#9470): the
/// rotating class except half-width katakana and Hangul, which keep their rotated setting.
fn upright_roman(c: char) -> bool {
    vertical_class(c) == VClass::Rotate && !(0xFF61..=0xFFDC).contains(&(c as u32))
}

/// The shaped advance (px, letter spacing included) of the cluster starting each of `ranges`
/// (sorted byte ranges of the shaped text), `None` where no cluster starts there.
fn cluster_advances(layout: &Layout<RunBrush>, ranges: &[(Range<usize>, f32, f32, usize)]) -> Vec<Option<f32>> {
    let mut out = vec![None; ranges.len()];
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            for c in gr.run().visual_clusters() {
                if let Ok(i) = ranges.binary_search_by_key(&c.text_range().start, |r| r.0.start) {
                    out[i] = Some(c.advance());
                }
            }
        }
    }
    out
}
