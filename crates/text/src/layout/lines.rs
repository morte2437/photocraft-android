//! Placing one broken line of a paragraph: kerning and tab stops, glyph and cluster positions,
//! decorations, and the line's vertical-type and justification adjustments.

use parley::{PositionedLayoutItem, Run};
use photocraft_doc::text::{CharStyle, Kerning, TextAlign};
use skrifa::instance::NormalizedCoord;

use super::area::{LineMetrics, line_metrics};
use super::shaping::Para;
use super::style::{first_ascent, squeeze_punctuation, synthetic_bold};
use super::vertical::{VGlyph, VMetrics};
use super::{ClusterInfo, DecorationRect, Flow, GlyphFace, GlyphOrient, LineInfo, PlacedGlyph, RunBrush, Slot, TextLayout, VClass};

type Line<'a> = parley::Line<'a, RunBrush>;
type GlyphRun<'a> = parley::GlyphRun<'a, RunBrush>;

/// A cluster of a line in visual order, for kerning.
struct KernSlot {
    /// Index of the run in the paragraph layout.
    run: usize,
    /// Byte offset in the paragraph text.
    start: usize,
    first: Option<u32>,
    last: Option<u32>,
    font: parley::FontData,
    coords: Vec<i16>,
    size: f32,
    rtl: bool,
    blank: bool,
    /// Advance along the line (px, horizontal scale included).
    advance: f32,
    /// What horizontal scale adds to the shaped advance (px; negative below 100%).
    growth: f32,
    /// A tab, shaped as a space; it advances to the next tab stop.
    tab: bool,
    /// Vertical type, upright glyph (its outline doesn't run along the column).
    upright: bool,
}

/// Where a line sits in its paragraph and on the page.
pub(super) struct LineFrame {
    pub(super) index: usize,
    pub(super) last: bool,
    /// The slot the area breaker gave this line.
    pub(super) slot: Option<Slot>,
    pub(super) baseline: f32,
    pub(super) ascent: f32,
    pub(super) descent: f32,
}

/// The clusters of one run, for applying kerning as glyphs are emitted.
struct KernCursor {
    run: usize,
    /// Slot of the cluster each glyph of the run belongs to.
    glyph_slot: Vec<usize>,
    /// Glyphs emitted so far.
    emitted: usize,
    /// First slot whose kerning isn't applied yet.
    applied: usize,
}

impl KernCursor {
    /// Kerning of the clusters before the next glyph's cluster.
    fn before_glyph(&mut self, kern_px: &[f32], extra: &mut f32) {
        if let Some(&slot) = self.glyph_slot.get(self.emitted) {
            while self.applied < slot {
                *extra += kern_px.get(self.applied).copied().unwrap_or(0.0);
                self.applied += 1;
            }
        }
        self.emitted += 1;
    }

    /// After the run's last glyph: its remaining clusters' kerning follows it.
    fn after_run(&mut self, slots: &[KernSlot], kern_px: &[f32], extra: &mut f32) {
        if self.emitted < self.glyph_slot.len() {
            return;
        }
        let end = slots.iter().rposition(|s| s.run == self.run).map_or(self.applied, |p| p + 1);
        while self.applied < end {
            *extra += kern_px.get(self.applied).copied().unwrap_or(0.0);
            self.applied += 1;
        }
    }
}

/// What is accumulated while a line's glyph runs are emitted.
struct LineState {
    cursors: Vec<KernCursor>,
    seen_runs: Vec<usize>,
    vinfo: Vec<VGlyph>,
    /// Horizontal-scale growth and kerning along the line.
    extra: f32,
}

/// Fixed facts about the line being emitted.
struct LineCtx<'a> {
    dx: f32,
    baseline: f32,
    slots: &'a [KernSlot],
    kern_px: &'a [f32],
    line_index: usize,
}

/// The horizontal extent of one glyph run on the line.
struct RunSpan {
    x0: f32,
    x1: f32,
    style: u32,
}

/// A run style's horizontal scale; unset (zero or negative) is 100%.
fn scale_of(st: &CharStyle) -> f32 {
    if st.horizontal_scale > 0.0 { st.horizontal_scale } else { 1.0 }
}

/// The clusters of `line` in visual order, per run.
fn collect_kern_slots(line: &Line<'_>, para: &Para<'_>) -> Vec<KernSlot> {
    let env = para.env;
    let mut slots: Vec<KernSlot> = Vec::new();
    let mut line_runs: Vec<usize> = Vec::new();
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(gr) = item else {
            continue;
        };
        let run = gr.run();
        if line_runs.contains(&run.index()) {
            continue;
        }
        line_runs.push(run.index());
        let hs = env.styles.get(gr.style().brush.0 as usize).map_or(1.0, scale_of);
        for c in run.visual_clusters() {
            let mut gl = c.glyphs();
            let first = gl.next().map(|g| g.id);
            let last = gl.last().map(|g| g.id).or(first);
            slots.push(KernSlot {
                run: run.index(),
                start: c.text_range().start,
                first,
                last,
                font: run.font().clone(),
                coords: run.normalized_coords().to_vec(),
                size: run.font_size(),
                rtl: c.is_rtl(),
                blank: first.is_none() || c.is_space_or_nbsp() || c.text_range().end <= para.prefix.len(),
                upright: env.vertical && c.first_style().brush.1 != VClass::Rotate as u8,
                advance: c.advance() * hs,
                growth: c.advance() * (hs - 1.0),
                tab: para.tabs.binary_search(&c.text_range().start).is_ok(),
            });
        }
    }
    slots
}

/// Both characters of a pair kern optically: a mode change splits shaping runs, which ends
/// automatic kerning, as with Metrics.
fn optical_pair(st: &CharStyle, next: &CharStyle, a: &KernSlot, b: &KernSlot) -> bool {
    st.kerning == Kerning::Optical
        && next.kerning == Kerning::Optical
        && st.kern == 0.0
        && next.kern == 0.0
        && !a.rtl
        && !b.rtl
        && !a.blank
        && !b.blank
        && !a.upright
        && !b.upright
        && a.font.data.id() == b.font.data.id()
        && a.font.index == b.font.index
        && a.coords == b.coords
        && (a.size - b.size).abs() < 1e-3
}

/// Optical kerning between two adjacent clusters (font units, with the size adjustment), if the
/// font has the pair.
fn optical_kern(optical: &mut crate::optical::Cache, st: &CharStyle, a: &KernSlot, b: &KernSlot) -> Option<f32> {
    let (l, r) = (a.last?, b.first?);
    let coords: Vec<NormalizedCoord> = a.coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
    let k = optical.pair(a.font.data.id(), a.font.data.as_ref(), a.font.index, &coords, l, r)?;
    Some(k + crate::optical::size_adjust(st.size_pt))
}

/// One kerning cursor per run on `line`.
fn kern_cursors(line: &Line<'_>, slots: &[KernSlot]) -> Vec<KernCursor> {
    let mut cursors: Vec<KernCursor> = Vec::new();
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(gr) = item else {
            continue;
        };
        let run = gr.run();
        if cursors.iter().any(|c| c.run == run.index()) {
            continue;
        }
        let base = slots.iter().position(|s| s.run == run.index()).unwrap_or(0);
        cursors.push(KernCursor { run: run.index(), glyph_slot: glyph_slots(run, base), emitted: 0, applied: base });
    }
    cursors
}

/// For each glyph of `run`, the slot of its cluster (the run's clusters start at slot `base`).
fn glyph_slots(run: &Run<'_, RunBrush>, base: usize) -> Vec<usize> {
    let mut g2s = Vec::new();
    for (ci, c) in run.visual_clusters().enumerate() {
        g2s.extend(c.glyphs().map(|_| base + ci));
    }
    g2s
}

/// Distributes `slack` between the clusters of the line from `c0` (and their glyphs from `g0`);
/// returns what was added to the line's length.
fn justify_line(out: &mut TextLayout, c0: usize, g0: usize, slack: f32) -> f32 {
    let n = out.clusters.len() - c0;
    if n <= 1 || slack <= 0.0 {
        return 0.0;
    }
    let step = slack / (n - 1) as f32;
    let starts: Vec<f32> = out.clusters[c0..].iter().map(|c| c.x).collect();
    for (i, c) in out.clusters[c0..].iter_mut().enumerate() {
        c.x += step * i as f32;
    }
    for g in &mut out.glyphs[g0..] {
        let i = starts.iter().rposition(|&s| s <= g.x + 1e-3).unwrap_or(0);
        g.x += step * i as f32;
    }
    slack
}

/// Line space → text space (90° clockwise); upright glyphs centred on the column.
fn rotate_to_text_space(glyphs: &mut [PlacedGlyph], vinfo: &[VGlyph], baseline: f32) {
    for (g, v) in glyphs.iter_mut().zip(vinfo) {
        let (u, cross) = (g.x, g.y);
        if v.upright {
            g.x = -baseline - v.advance * 0.5;
            g.y = u + v.origin;
            g.orient = GlyphOrient::Upright;
        } else {
            g.x = -(cross + v.centre);
            g.y = u;
            g.orient = GlyphOrient::Rotated;
        }
    }
}

/// Underline and strikethrough rects of one run.
fn push_decorations(out: &mut TextLayout, vertical: bool, gr: &GlyphRun<'_>, baseline: f32, shift: f32, span: &RunSpan) {
    let rm = gr.run().metrics();
    let (x0, x1, si) = (span.x0, span.x1, span.style);
    if vertical {
        // Underline to the right of the column, strikethrough through its centre
        // (stored directly in text space).
        let half = gr.run().font_size() * 0.5;
        let mut push = |v0: f32, v1: f32| out.decorations.push(DecorationRect { x0: -v1, y0: x0, x1: -v0, y1: x1, style: si });
        if gr.style().underline.is_some() {
            let v1 = baseline - half;
            push(v1 - rm.underline_size.max(1.0), v1);
        }
        if gr.style().strikethrough.is_some() {
            let sz = rm.strikethrough_size.max(1.0);
            push(baseline - sz * 0.5, baseline + sz * 0.5);
        }
        return;
    }
    if gr.style().underline.is_some() {
        let y0 = baseline - rm.underline_offset - shift;
        out.decorations.push(DecorationRect { x0, y0, x1, y1: y0 + rm.underline_size.max(1.0), style: si });
    }
    if gr.style().strikethrough.is_some() {
        let y0 = baseline - rm.strikethrough_offset - shift;
        out.decorations.push(DecorationRect { x0, y0, x1, y1: y0 + rm.strikethrough_size.max(1.0), style: si });
    }
}

impl Para<'_> {
    /// Kerning after each cluster (px): manual kerning plus optical pair kerning. Applied after
    /// shaping (like horizontal scale) in line space, so it moves the following glyphs, carets and
    /// the line extent; in vertical type that is along the column. Optical kerning measures
    /// horizontal outlines, so it applies to rotated (Latin) glyphs only there.
    fn kerning(&self, optical: &mut crate::optical::Cache, slots: &[KernSlot]) -> Vec<f32> {
        let env = self.env;
        let mut kern_px: Vec<f32> = vec![0.0; slots.len()];
        for j in 0..slots.len().saturating_sub(1) {
            let (a, b) = (&slots[j], &slots[j + 1]);
            if a.start < self.prefix.len() {
                continue;
            }
            let st = &env.styles[env.style_at(self.map_offset(a.start))];
            let next = &env.styles[env.style_at(self.map_offset(b.start))];
            let mut units = if st.kern.is_finite() { st.kern } else { 0.0 };
            if optical_pair(st, next, a, b) {
                units += optical_kern(optical, st, a, b).unwrap_or(0.0);
            }
            kern_px[j] = units / 1000.0 * a.size;
        }
        kern_px
    }

    /// Tabs: the "kerning" after a tab takes the pen to the next stop, measured from the anchor or
    /// the box edge as if the line were left-aligned (alignment then moves the whole line).
    /// Returns the tab width and horizontal-scale growth in the line's trailing whitespace, which
    /// is outside its extent.
    fn apply_tab_stops(&self, slots: &[KernSlot], kern_px: &mut [f32], line_index: usize) -> (f32, f32) {
        let mut pos = self.indent_start + if line_index == 0 { self.first_indent } else { 0.0 };
        let mut trailing_tabs = 0.0f32;
        let mut trailing_growth = 0.0f32;
        for (j, s) in slots.iter().enumerate() {
            let rest_blank = slots[j + 1..].iter().all(|n| n.blank);
            if s.blank && rest_blank {
                trailing_growth += s.growth;
            }
            if s.tab {
                kern_px[j] = super::next_tab_stop(pos, self.tab_px) - pos - s.advance;
                if rest_blank {
                    trailing_tabs += kern_px[j];
                }
            }
            pos += s.advance + kern_px.get(j).copied().unwrap_or(0.0);
        }
        (trailing_tabs, trailing_growth)
    }

    /// Where the line starts along its extent: box and area lines at their box or span edge,
    /// point lines around the anchor.
    fn line_start(&self, f: &LineFrame, offset: f32, adv: f32, line_kern: f32) -> f32 {
        if !self.env.wraps {
            let target = match self.ps.align {
                TextAlign::Center | TextAlign::JustifyCenter => -(adv + line_kern) / 2.0,
                TextAlign::Right | TextAlign::JustifyRight => -(adv + line_kern),
                _ => self.indent_start,
            };
            return target - offset;
        }
        // Parley aligned box lines without the kerning.
        let kern_align = match self.ps.align {
            TextAlign::Center => line_kern / 2.0,
            TextAlign::Right => line_kern,
            _ => 0.0,
        };
        let origin = f.slot.map_or(self.line_origin, |s| s.x);
        let base = origin + self.indent_start - kern_align;
        // The line's own width takes the justify-last-line slack.
        let slack = self.line_avail(f).unwrap_or(0.0) - adv * self.break_scale - line_kern;
        // Parley aligned the unscaled line in `avail / break_scale`: scale its
        // indent and alignment offset back to the box.
        let scaled_offset = offset * (self.break_scale - 1.0);
        base + if f.last {
            match self.ps.align {
                TextAlign::JustifyCenter => slack * 0.5 - offset,
                TextAlign::JustifyRight => slack - offset,
                _ => scaled_offset,
            }
        } else {
            scaled_offset
        }
    }

    /// The width the line may fill: its area slot's, else the box's.
    fn line_avail(&self, f: &LineFrame) -> Option<f32> {
        f.slot.map(|s| s.width).or(self.avail)
    }

    /// Where line `index` of `nlines` goes, advancing `flow` past it; `None` if a box clips it.
    pub(super) fn frame_line(&self, line: &Line<'_>, index: usize, nlines: usize, slot: Option<Slot>, flow: &mut Flow) -> Option<LineFrame> {
        let env = self.env;
        let LineMetrics { ascent, descent, leading } = line_metrics(line, env.styles, &self.ps, env.k, self.first_style(), env.vertical);
        let (box_x, box_y, box_w, box_h) = env.box_rect;
        let baseline = match (slot, flow.prev_baseline) {
            (Some(s), _) => s.baseline,
            // The first column's right edge touches the box's right edge.
            // (In vertical type the ascent is half the column width.)
            (None, None) if env.is_box && env.vertical => -(box_x + box_w) + ascent,
            // Photoshop's "first baseline: ascent": the top of the tallest ascender
            // (height of 'd') touches the box top, not the font's hhea ascent.
            (None, None) if env.is_box => box_y + first_ascent(line).unwrap_or(ascent),
            (None, None) => 0.0,
            (None, Some(b)) => b + leading + flow.pending_space,
        };
        flow.pending_space = 0.0;
        // Photoshop keeps a horizontal line whose baseline is inside the box and lets
        // its descenders hang below the bottom edge (#1317: three 35 pt lines in a
        // 91.3 pt box, the last baseline at 88). Requiring the descent to fit dropped
        // that line.
        // Photoshop keeps a horizontal line whose baseline is inside the box and lets its
        // descenders hang below the bottom edge (#1317: three 35 pt lines in a 91.3 pt box, the
        // last baseline at 88). Requiring the descent to fit dropped that line.
        let overflow = if env.vertical { baseline + descent > -box_x + 0.5 } else { baseline > box_y + box_h + 0.5 };
        if env.is_box && overflow {
            return None;
        }
        flow.prev_baseline = Some(baseline);
        Some(LineFrame { index, last: index + 1 == nlines, slot, baseline, ascent, descent })
    }

    /// Appends one line's clusters, glyphs, decorations and `LineInfo` to `out`.
    pub(super) fn place_line(&self, optical: &mut crate::optical::Cache, out: &mut TextLayout, line: &Line<'_>, f: &LineFrame) {
        let env = self.env;
        let m = *line.metrics();
        let adv = m.advance - m.trailing_whitespace;
        let slots = collect_kern_slots(line, self);
        let mut kern_px = self.kerning(optical, &slots);
        let (trailing_tabs, trailing_growth) = self.apply_tab_stops(&slots, &mut kern_px, f.index);
        let line_kern: f32 = kern_px.iter().sum::<f32>() - trailing_tabs;
        let dx = self.line_start(f, m.offset, adv, line_kern);
        let lc = LineCtx { dx, baseline: f.baseline, slots: &slots, kern_px: &kern_px, line_index: out.lines.len() };
        let (g0, c0) = (out.glyphs.len(), out.clusters.len());
        let mut state = LineState { cursors: kern_cursors(line, &slots), seen_runs: Vec::new(), vinfo: Vec::new(), extra: 0.0 };
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(gr) = item {
                self.emit_run(out, &lc, &mut state, &gr);
            }
        }
        if env.vertical {
            state.extra -= squeeze_punctuation(env.text, &mut out.clusters[c0..], &mut out.glyphs[g0..]);
        }
        if env.wraps && f.last && self.ps.align == TextAlign::JustifyAll {
            let slack = self.line_avail(f).unwrap_or(0.0) - (adv + state.extra);
            state.extra += justify_line(out, c0, g0, slack);
        }
        if env.vertical {
            rotate_to_text_space(&mut out.glyphs[g0..], &state.vinfo, f.baseline);
        }
        let lr = line.text_range();
        let x0 = dx + m.offset;
        out.lines.push(LineInfo {
            range: self.map_offset(lr.start)..self.map_offset(lr.end).min(self.content_end),
            baseline: f.baseline,
            x0,
            x1: x0 + adv + state.extra - trailing_tabs - trailing_growth,
            ascent: f.ascent,
            descent: f.descent,
            paragraph: self.index,
        });
    }

    /// Emits one glyph run of the line: its face, clusters (once per shaping run), glyphs and
    /// decorations.
    fn emit_run(&self, out: &mut TextLayout, lc: &LineCtx<'_>, state: &mut LineState, gr: &GlyphRun<'_>) {
        let env = self.env;
        let run = gr.run();
        let si = gr.style().brush.0;
        let st = &env.styles[si as usize];
        let hs = scale_of(st);
        let synth = run.synthesis();
        let face = out.faces.len() as u32;
        out.faces.push(GlyphFace {
            font: run.font().clone(),
            coords: run.normalized_coords().to_vec(),
            size_px: run.font_size(),
            embolden: synthetic_bold(synth.embolden(), st.weight, run.font_attrs().weight.value()),
            skew_deg: synth.skew().unwrap_or(0.0),
        });
        let run_x0 = lc.dx + gr.offset() + state.extra;
        if !state.seen_runs.contains(&run.index()) {
            state.seen_runs.push(run.index());
            self.push_clusters(out, lc, run, hs, run_x0);
        }
        self.push_glyphs(out, lc, state, gr, face, hs);
        let span = RunSpan { x0: run_x0, x1: lc.dx + gr.offset() + gr.advance() + state.extra, style: si };
        push_decorations(out, env.vertical, gr, lc.baseline, st.baseline_shift_pt * env.k, &span);
    }

    /// The caret clusters of a shaping run, with kerning and horizontal scale in their advances.
    fn push_clusters(&self, out: &mut TextLayout, lc: &LineCtx<'_>, run: &Run<'_, RunBrush>, hs: f32, run_x0: f32) {
        let mut cx = run_x0;
        let base = lc.slots.iter().position(|s| s.run == run.index()).unwrap_or(0);
        for (ci, c) in run.visual_clusters().enumerate() {
            let a = c.advance() * hs + lc.kern_px.get(base + ci).copied().unwrap_or(0.0);
            let r = c.text_range();
            if r.end > self.prefix.len() || self.prefix.is_empty() {
                out.clusters.push(ClusterInfo {
                    range: self.map_offset(r.start)..self.map_offset(r.end),
                    x: cx,
                    advance: a,
                    line: lc.line_index,
                    rtl: c.is_rtl(),
                });
            }
            cx += a;
        }
    }

    /// The glyphs of one run, with the kerning of the clusters before each.
    fn push_glyphs(&self, out: &mut TextLayout, lc: &LineCtx<'_>, state: &mut LineState, gr: &GlyphRun<'_>, face: u32, hs: f32) {
        let run = gr.run();
        let rm = run.metrics();
        let vm = if self.env.vertical { Some(VMetrics::new(run.font(), run.normalized_coords(), run.font_size())) } else { None };
        let class = gr.style().brush.1;
        // Glyphs of the run's own characters (not substituted by `vert`).
        let plain: Vec<u32> = match (&vm, class) {
            (Some(v), 2) => self.text_of(run.text_range()).chars().filter_map(|c| v.cmap(c)).collect(),
            _ => Vec::new(),
        };
        let mut pen = gr.offset();
        let mut cursor = state.cursors.iter_mut().find(|c| c.run == run.index());
        for g in gr.glyphs() {
            if let Some(c) = cursor.as_deref_mut() {
                c.before_glyph(lc.kern_px, &mut state.extra);
            }
            out.glyphs.push(PlacedGlyph {
                face,
                id: g.id,
                x: lc.dx + pen + g.x + state.extra,
                y: lc.baseline + g.y,
                style: gr.style().brush.0,
                orient: GlyphOrient::Horizontal,
            });
            if let Some(v) = &vm {
                let upright = class == 1 || (class == 2 && !plain.contains(&g.id));
                state.vinfo.push(VGlyph {
                    upright,
                    advance: g.advance * hs,
                    origin: if upright { v.origin(g.id) } else { 0.0 },
                    // Rotated text is centred on the column: its baseline sits
                    // (ascent − descent) / 2 from the centre line.
                    centre: (rm.ascent - rm.descent) * 0.5,
                });
            }
            state.extra += g.advance * (hs - 1.0);
            pen += g.advance;
        }
        if let Some(c) = cursor {
            c.after_run(lc.slots, lc.kern_px, &mut state.extra);
        }
    }
}
