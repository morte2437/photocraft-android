//! Shaping and layout of a text layer's model into positioned glyphs (text-space pixels, y down,
//! first baseline of point text at y = 0).
//!
//! Each paragraph is shaped and line-broken by parley (HarfRust shaping, Unicode bidi and line
//! breaking); paragraphs are then stacked with Photoshop's rules: baseline-to-baseline distance =
//! the largest leading on the line (auto leading = paragraph factor × size), space before/after,
//! indents, and point-text alignment around the anchor.
//!
//! **Vertical type** (tategaki) is laid out in the same *line space* and then turned 90° clockwise
//! into text space: a line becomes a column running top to bottom, and successive columns advance
//! right to left. Within a column, CJK characters stay upright (centred on the column, placed with
//! the font's vertical metrics: `VORG`, `vhea`/`vmtx`, else the ideographic em box) and use the
//! font's vertical alternates (`vert`) for punctuation, brackets and the long-vowel mark. Roman
//! letters stand upright one per em unless the run sets `rotate_roman`. Lines, clusters, carets
//! and hit tests stay in line space; [`TextLayout::to_text`] / [`TextLayout::to_line`] convert.

use std::borrow::Cow;
use std::ops::Range;

use parley::{FontData, FontFamily, FontFeatures, FontStyle, FontVariations, FontWeight, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{Caps, CharStyle, FontBaseline, Kerning, Orientation, ParagraphStyle, TextShape};
use skrifa::raw::types::Tag;

use crate::fonts::FontDb;

mod area;
mod lines;
mod navigate;
mod paragraphs;
mod shaping;
mod style;
mod vertical;

#[cfg(test)]
pub(crate) use area::AREA_PASSES;
pub use navigate::{byte_index, char_index, hit_char, line_edge, line_index, line_step, text_point_inside, word_boundary};
use paragraphs::next_tab_stop;
pub use paragraphs::{DEFAULT_TAB_STOP_PT, split_paragraphs};
use shaping::{Para, resolve_run_styles, running_starts, small_caps_modes};
use style::SmallCapsMode;
#[cfg(test)]
use style::{SMCP_PROBES, feature_list, small_caps_mode, variation_list};
#[cfg(test)]
pub(crate) use style::{hangul_runs, squeeze_punctuation, synthetic_bold};
pub use vertical::{VClass, vertical_class};

/// Index of the run containing byte offset `off`, given each run's start offset (non-decreasing):
/// the last run starting at or before `off`, or 0 when none does. Binary search, so a layer with
/// many runs doesn't make per-character lookups quadratic.
fn run_index_at(run_starts: &[usize], off: usize) -> usize {
    run_starts.partition_point(|&s| s <= off).saturating_sub(1)
}

/// Index of the character run whose style a glyph uses, plus the vertical-type class of its
/// characters ([`VClass`] as `u8`; always 0 in horizontal type).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunBrush(pub u32, pub u8);

/// How a placed glyph's outline is oriented in text space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphOrient {
    /// Horizontal type: origin on the baseline, outline upright.
    #[default]
    Horizontal,
    /// Vertical type, upright: the origin is the glyph's horizontal origin (left end of its
    /// advance, on its baseline); baseline shift moves it right.
    Upright,
    /// Vertical type, rotated 90° clockwise about its origin (the baseline runs downwards).
    Rotated,
}

/// A font instance used by some glyphs.
#[derive(Clone, Debug)]
pub struct GlyphFace {
    pub font: FontData,
    /// Normalised variation coordinates (F2Dot14 bits).
    pub coords: Vec<i16>,
    pub size_px: f32,
    /// Synthetic bold requested by font matching (face lacks the weight).
    pub embolden: bool,
    /// Synthetic oblique angle in degrees (face lacks italics).
    pub skew_deg: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub face: u32,
    pub id: u32,
    /// Pen position (text space px): x along the baseline, y = baseline (before baseline shift).
    pub x: f32,
    pub y: f32,
    /// Character run index (into [`TextLayout::styles`]).
    pub style: u32,
    /// Orientation of the outline (vertical type).
    pub orient: GlyphOrient,
}

/// Underline/strikethrough rectangle (text space px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub style: u32,
}

/// A line (a column in vertical type) in line space: `x` runs along the line and `baseline` is the
/// cross-line position. In vertical type `baseline` is the column's centre line and
/// `ascent`/`descent` are the column's half widths on either side.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    /// Byte range in the layer text (without the paragraph break).
    pub range: Range<usize>,
    pub baseline: f32,
    /// Visual extent along the baseline.
    pub x0: f32,
    pub x1: f32,
    pub ascent: f32,
    pub descent: f32,
    pub paragraph: usize,
}

/// A grapheme cluster (caret stops, hit testing, selection), in line space.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterInfo {
    pub range: Range<usize>,
    pub x: f32,
    pub advance: f32,
    pub line: usize,
    pub rtl: bool,
}

/// Result of laying out a text layer.
#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub faces: Vec<GlyphFace>,
    pub glyphs: Vec<PlacedGlyph>,
    pub decorations: Vec<DecorationRect>,
    pub lines: Vec<LineInfo>,
    pub clusters: Vec<ClusterInfo>,
    /// Resolved character styles (one per run of the layer).
    pub styles: Vec<CharStyle>,
    /// Pixels per point used (dpi / 72).
    pub px_per_pt: f32,
    /// Vertical type: line space is turned 90° clockwise into text space.
    pub vertical: bool,
}

impl TextLayout {
    /// Line space → text space: the identity for horizontal type; for vertical type the line
    /// direction points down and the cross-line direction (towards the next line) points left.
    pub fn to_text(&self, x: f32, y: f32) -> (f32, f32) {
        if self.vertical { (-y, x) } else { (x, y) }
    }

    /// Text space → line space (inverse of [`Self::to_text`]).
    pub fn to_line(&self, x: f32, y: f32) -> (f32, f32) {
        if self.vertical { (y, -x) } else { (x, y) }
    }

    /// Logical bounds (x0, y0, x1, y1) of all lines, text space.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let b = self.line_bounds()?;
        Some(if self.vertical { [-b[3], b[0], -b[1], b[2]] } else { b })
    }

    /// Logical bounds of all lines in line space (equal to [`Self::bounds`] for horizontal type).
    pub fn line_bounds(&self) -> Option<[f32; 4]> {
        self.lines.iter().fold(None, |acc, l| {
            let r = [l.x0, l.baseline - l.ascent, l.x1, l.baseline + l.descent];
            Some(match acc {
                None => r,
                Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
            })
        })
    }

    /// Whether boxed text doesn't all fit its shape: a box's layout drops the lines that
    /// overflow (`layout` breaks out of the line loop), so any character the laid-out lines
    /// don't cover — the last line's range ends short of the whole text — points at dropped
    /// content. Text that fits completely covers its end: paragraph breaks are inside their
    /// lines, and a trailing empty paragraph that fits gains its own (empty) line. Empty text
    /// has nothing to drop. Callers gate this on the shape being a box; point text has no box
    /// to overflow.
    pub fn overflows(&self, text: &str) -> bool {
        !text.is_empty() && self.lines.last().is_none_or(|ln| ln.range.end < text.len())
    }

    /// Byte offset of the caret nearest to a text-space point.
    pub fn hit_test(&self, x: f32, y: f32) -> usize {
        let (x, y) = self.to_line(x, y);
        self.hit_test_line(x, y)
    }

    /// The visual line nearest to a line-space ordinate, including its ascent and descent.
    /// Pointer hit testing uses this even where adjacent lines share a caret byte offset.
    fn closest_line(&self, y: f32) -> Option<(usize, &LineInfo)> {
        self.lines.iter().enumerate().min_by(|a, b| {
            let d = |l: &LineInfo| {
                if y < l.baseline - l.ascent {
                    l.baseline - l.ascent - y
                } else if y > l.baseline + l.descent {
                    y - l.baseline - l.descent
                } else {
                    0.0
                }
            };
            d(a.1).total_cmp(&d(b.1))
        })
    }

    /// Byte offset of the caret nearest to a line-space point.
    pub fn hit_test_line(&self, x: f32, y: f32) -> usize {
        let Some((li, _)) = self.closest_line(y) else {
            return 0;
        };
        self.hit_test_x_on_line(li, x)
    }

    fn hit_test_x_on_line(&self, line_index: usize, x: f32) -> usize {
        let Some(line) = self.lines.get(line_index) else { return 0 };
        let mut best = (f32::MAX, line.range.end);
        for c in self.clusters.iter().filter(|c| c.line == line_index) {
            let mid = c.x + c.advance / 2.0;
            let (before, after) = if c.rtl { (c.range.end, c.range.start) } else { (c.range.start, c.range.end) };
            let (dist, off) = if x < mid { ((x - c.x).abs(), before) } else { ((x - c.x - c.advance).abs(), after) };
            if dist < best.0 {
                best = (dist, off);
            }
        }
        best.1
    }

    /// The caret for a byte offset as a text-space segment (its two end points).
    pub fn caret_segment(&self, offset: usize) -> [(f32, f32); 2] {
        let (x, top, bottom) = self.caret(offset);
        [self.to_text(x, top), self.to_text(x, bottom)]
    }

    /// Caret geometry for a byte offset: (x, top, bottom) in line space (the same as text space
    /// for horizontal type; see [`Self::caret_segment`]).
    pub fn caret(&self, offset: usize) -> (f32, f32, f32) {
        for c in &self.clusters {
            if c.range.start == offset
                && let Some(l) = self.lines.get(c.line)
            {
                let x = if c.rtl { c.x + c.advance } else { c.x };
                return (x, l.baseline - l.ascent, l.baseline + l.descent);
            }
        }
        // End of a line (or empty line): after the last cluster of the line containing it.
        let li = self.lines.iter().position(|l| offset >= l.range.start && offset <= l.range.end).unwrap_or(self.lines.len().saturating_sub(1));
        match self.lines.get(li) {
            Some(l) => {
                let x = self
                    .clusters
                    .iter()
                    .filter(|c| c.line == li && c.range.end == offset)
                    .map(|c| if c.rtl { c.x } else { c.x + c.advance })
                    .next()
                    .unwrap_or(l.x1);
                (x, l.baseline - l.ascent, l.baseline + l.descent)
            }
            None => (0.0, 0.0, 0.0),
        }
    }
}

const LRM: &str = "\u{200E}";
const RLM: &str = "\u{200F}";
/// A line break inside a paragraph (Shift+Return), stored as U+0003 in PSD type.
pub(crate) const FORCED_LINE_BREAK: char = '\u{3}';

pub(crate) struct Layouter {
    lcx: LayoutContext<RunBrush>,
    optical: crate::optical::Cache,
}

impl Layouter {
    pub fn new() -> Self {
        Self { lcx: LayoutContext::new(), optical: crate::optical::Cache::default() }
    }

    pub fn layout(&mut self, fonts: &mut FontDb, t: &TextLayer, dpi: f32) -> TextLayout {
        let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
        let runs = t.char_runs();
        let paras = t.paragraph_runs();
        let vertical = t.orientation == Orientation::Vertical;
        let mut out = TextLayout { px_per_pt: k, vertical, ..Default::default() };
        let styles = resolve_run_styles(fonts, &runs);
        let run_starts = running_starts(runs.iter().map(|r| r.len));
        let para_starts = running_starts(paras.iter().map(|r| r.len));
        let (box_rect, is_box) = match t.shape {
            TextShape::Box { x, y, width, height } => ((x, y, width, height), true),
            // Path text is laid out as one point-text line, then bent onto the path. Area text
            // places its own lines (`area` below).
            TextShape::Point | TextShape::Path { .. } | TextShape::Area { .. } => ((0.0, 0.0, 0.0, 0.0), false),
        };
        // Area type: the shape its lines wrap inside. A path that encloses nothing (only from a
        // hostile file; `type.fillPath` refuses one) lays out as point text.
        let area = match &t.shape {
            TextShape::Area { path } => crate::area::AreaShape::new(path, vertical),
            _ => None,
        };
        let small_caps = small_caps_modes(fonts, &styles);
        let env = Env {
            text: &t.text,
            runs: &runs,
            styles: &styles,
            run_starts: &run_starts,
            small_caps: &small_caps,
            k,
            vertical,
            is_box,
            // Box and area text wrap, align within their lines and hide what doesn't fit.
            wraps: is_box || area.is_some(),
            box_rect,
            area,
        };
        let mut flow = Flow::default();
        for (pi, prange) in split_paragraphs(&t.text).into_iter().enumerate() {
            if flow.stop {
                break;
            }
            let ps = paras[para_starts.iter().rposition(|&s| s <= prange.start).unwrap_or(0)].style.clone();
            self.layout_paragraph(fonts, &mut out, &mut flow, Para::new(&env, fonts, pi, prange, ps));
        }
        out.styles = styles;
        out
    }

    /// Breaks one paragraph and stacks its lines below the previous paragraph's.
    fn layout_paragraph(&mut self, fonts: &mut FontDb, out: &mut TextLayout, flow: &mut Flow, para: Para<'_>) {
        let env = para.env;
        let (mut layout, area_slots, area_full) = para.break_paragraph(&mut self.lcx, &mut fonts.fcx, flow);
        layout.align(para.alignment(), para.align_options());
        if flow.prev_baseline.is_some() {
            flow.pending_space += para.ps.space_before_pt * env.k;
        }
        let nlines = layout.len();
        for (li, line) in layout.lines().enumerate() {
            // Area type: the slot the breaker gave this line.
            let slot = area_slots.get(li).copied();
            if env.area.is_some() && slot.is_none() {
                flow.stop = true;
                break;
            }
            let Some(frame) = para.frame_line(&line, li, nlines, slot, flow) else {
                flow.stop = true;
                break;
            };
            para.place_line(&mut self.optical, out, &line, &frame);
        }
        if area_full {
            // The shape is full: later paragraphs are hidden too.
            flow.stop = true;
        }
        flow.pending_space += para.ps.space_after_pt * env.k;
    }
}

/// What a layout run shares between its paragraphs.
struct Env<'a> {
    text: &'a str,
    runs: &'a [photocraft_doc::text::TextRun],
    styles: &'a [CharStyle],
    /// Start offset of each run.
    run_starts: &'a [usize],
    small_caps: &'a [SmallCapsMode],
    k: f32,
    vertical: bool,
    is_box: bool,
    /// Box and area text wrap, align within their lines and hide what doesn't fit.
    wraps: bool,
    box_rect: (f32, f32, f32, f32),
    area: Option<crate::area::AreaShape>,
}

impl Env<'_> {
    /// Index of the run (and style) containing byte offset `off`.
    fn style_at(&self, off: usize) -> usize {
        run_index_at(self.run_starts, off)
    }
}

/// Where the previous paragraph left off.
#[derive(Default)]
struct Flow {
    prev_baseline: Option<f32>,
    pending_space: f32,
    stop: bool,
}

/// Where one line of area type goes: its baseline, the left edge of its span, and the width
/// it may fill (the span less the paragraph's indents), plus the row and span it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    baseline: f32,
    x: f32,
    width: f32,
    row: usize,
    span: usize,
}

#[cfg(test)]
mod tests {
    use super::{SmallCapsMode, feature_list, small_caps_mode, variation_list};
    use photocraft_doc::TextLayer;
    use photocraft_doc::text::{Caps, CharStyle, FontVariation, TextRun};

    /// Reference: the lookup the layout used before it became a binary search.
    fn run_index_reference(starts: &[usize], off: usize) -> usize {
        starts.iter().rposition(|&s| s <= off).unwrap_or(0)
    }

    #[test]
    fn run_lookup_matches_the_linear_reference() {
        // Deterministic pseudo-random run layouts with empty runs, equal adjacent styles and
        // offsets past the end.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % m) as usize
        };
        for _ in 0..300 {
            let n = next(12);
            let mut acc = 0;
            let starts: Vec<usize> = (0..n)
                .map(|_| {
                    let s = acc;
                    acc += next(4); // 0 = empty run
                    s
                })
                .collect();
            for off in 0..acc + 4 {
                assert_eq!(super::run_index_at(&starts, off), run_index_reference(&starts, off), "{starts:?} @ {off}");
            }
        }
        assert_eq!(super::run_index_at(&[], 5), 0);
    }

    /// Many runs and paragraphs: the font is probed once per small-caps style per layout, not
    /// once per style per paragraph, and styles that don't use small caps are never probed.
    #[test]
    fn small_caps_probe_runs_once_per_small_caps_style() {
        let plain = CharStyle { font_family: "Inter".into(), ..Default::default() };
        let small = CharStyle { caps: Caps::SmallCaps, ..plain.clone() };
        let text: String = "a\n".repeat(300);
        let runs = |small_every: usize| -> Vec<TextRun> {
            (0..text.len()).map(|i| TextRun { len: 1, style: if small_every > 0 && i % small_every == 0 { small.clone() } else { plain.clone() } }).collect()
        };
        let mut engine = crate::TextEngine::new();
        let mut probes = |small_every: usize| {
            super::SMCP_PROBES.with(|c| c.set(0));
            let layer = TextLayer { text: text.clone(), runs: runs(small_every), ..Default::default() };
            let _ = engine.layout(&layer, 72.0);
            super::SMCP_PROBES.with(|c| c.get())
        };
        assert_eq!(probes(0), 0);
        let styles = text.len().div_ceil(2); // every other index is a small-caps run
        assert_eq!(probes(2), styles);
    }

    /// A variable font's optical size follows the type size unless the style sets `opsz` (#2985).
    #[test]
    fn optical_size_follows_the_type_size_unless_set() {
        let style = CharStyle { size_pt: 36.0, variations: vec![FontVariation { axis: "wght".into(), value: 650.0 }], ..Default::default() };
        assert_eq!(variation_list(&style), ["\"wght\" 650", "\"opsz\" 36"]);
        let set = CharStyle { size_pt: 36.0, variations: vec![FontVariation { axis: "opsz".into(), value: 12.0 }], ..Default::default() };
        assert_eq!(variation_list(&set), ["\"opsz\" 12"]);
    }

    #[test]
    fn uses_real_small_caps_only_when_the_selected_face_supports_smcp() {
        let style = CharStyle { caps: Caps::SmallCaps, ..Default::default() };
        let real = small_caps_mode(style.caps, true);
        let synthetic = small_caps_mode(style.caps, false);

        assert_eq!(real, SmallCapsMode::OpenType);
        assert!(feature_list(&style, real).iter().any(|feature| feature == "\"smcp\" 1"));
        assert_eq!(synthetic, SmallCapsMode::Synthetic);
        assert!(!feature_list(&style, synthetic).iter().any(|feature| feature == "\"smcp\" 1"));
    }
}
