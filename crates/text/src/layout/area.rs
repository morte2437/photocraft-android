use super::*;

/// A line's ascent, descent and leading (px); for area type, the largest over a paragraph.
#[derive(Clone, Copy, Debug)]
pub(super) struct LineMetrics {
    pub(super) ascent: f32,
    pub(super) descent: f32,
    pub(super) leading: f32,
}

/// Most line-breaking passes area type makes for one paragraph, over all its plans (each span
/// left empty for a word that doesn't fit costs a pass). Past it, words overflow their spans.
pub(super) const MAX_AREA_PASSES: usize = 32;

/// Most rows one area paragraph is planned on.
const MAX_AREA_ROWS: usize = 4096;

#[cfg(test)]
thread_local! {
    /// Area line-breaking passes made on this thread, for tests.
    pub(crate) static AREA_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The rows an area paragraph's lines go on: baselines from `next` down, one leading apart,
/// each with its spans left to right. They don't depend on the text, so they are found once
/// (lazily) and shared by every pass.
struct AreaRows<'a> {
    area: &'a crate::area::AreaShape,
    m: LineMetrics,
    min_width: f32,
    /// Where the search for the next row starts.
    next: f32,
    rows: Vec<(f32, Vec<(f32, f32)>)>,
    done: bool,
}

impl AreaRows<'_> {
    /// Row `i`: its baseline and spans; `None` past the shape.
    fn get(&mut self, i: usize) -> Option<&(f32, Vec<(f32, f32)>)> {
        while self.rows.len() <= i && !self.done {
            match self.area.next_row(self.next, self.m.ascent, self.m.descent, self.min_width) {
                Some((b, spans)) if self.rows.len() < MAX_AREA_ROWS => {
                    self.next = b + self.m.leading;
                    self.rows.push((b, spans));
                }
                _ => self.done = true,
            }
        }
        self.rows.get(i)
    }

    /// The first span, from span `span` of row `row` on, that leaves at least `width` once
    /// `inset` (the paragraph's indents) is taken off: its row and span.
    fn first_fit(&mut self, row: usize, span: usize, width: f32, inset: f32) -> Option<(usize, usize)> {
        let (mut r, mut first) = (row, span);
        while let Some((_, spans)) = self.get(r) {
            if let Some(j) = spans.iter().skip(first).position(|&(x0, x1)| x1 - x0 - inset + 0.5 >= width) {
                return Some((r, first + j));
            }
            r += 1;
            first = 0;
        }
        None
    }
}

/// A line wider than the span it fell in, that a later span can hold: the line's index in the
/// paragraph, where its text starts, and its width.
#[derive(Clone, Copy, Debug)]
struct Need {
    line: usize,
    start: usize,
    width: f32,
}

/// Breaks one paragraph of area type a line at a time, each line as wide as the next span the
/// shape leaves (rows from `from` down, spans left to right). Returns each line's slot and
/// whether text was left over because the shape ran out (hidden, like box overflow).
///
/// A span too narrow for the word that would start it (the narrow top of an ellipse) is left
/// empty and the word goes to the first later span that holds it, as Photoshop does. A word no
/// later span holds but another span of the shape would means the shape is full: it and the rest are
/// hidden. Only a word no span holds overflows its span. Parley doesn't report a line's
/// trailing whitespace while breaking, so overflowing lines are found after a pass and the
/// paragraph broken again; `budget` (shared by the paragraph's calls) bounds the passes.
///
/// Horizontal scale applies after shaping, so each line breaks at its span width divided by
/// `scale`; widths compared with spans are the scaled ones.
#[allow(clippy::too_many_arguments)]
pub(super) fn break_area(
    layout: &mut Layout<RunBrush>,
    area: &crate::area::AreaShape,
    from: f32,
    m: LineMetrics,
    indents: (f32, f32),
    min_width: f32,
    scale: f32,
    budget: &std::cell::Cell<usize>,
) -> (Vec<Slot>, bool) {
    let inset = indents.0 + indents.1;
    let mut rows = AreaRows { area, m, min_width, next: from, rows: Vec::new(), done: false };
    // Every row of the shape from its top, whatever paragraph came before (found lazily).
    let mut shape_rows = AreaRows { area, m, min_width, next: area.bounds[1], rows: Vec::new(), done: false };
    let mut needs: Vec<Need> = Vec::new();
    loop {
        let (slots, overflow) = break_area_pass(layout, &mut rows, inset, scale, &needs);
        budget.set(budget.get().saturating_sub(1));
        // Each line's text start and scaled width without its trailing whitespace.
        let lines: Vec<(usize, f32)> = layout
            .lines()
            .map(|l| {
                let lm = l.metrics();
                (l.text_range().start, (lm.advance - lm.trailing_whitespace) * scale)
            })
            .collect();
        // A line still wider than its span that some span of the shape would hold: the shape ran
        // out of room for it (the narrow bottom of a full bubble). It and the rest are hidden, as
        // overflow; only a word no span holds overflows its span.
        let mut ran_out = |slots: Vec<Slot>, overflow: bool| match lines
            .iter()
            .zip(&slots)
            .position(|(&(_, width), s)| width > s.width + 0.5 && shape_rows.first_fit(0, 0, width, inset).is_some())
        {
            Some(li) => (slots.into_iter().take(li).collect(), true),
            None => (slots, overflow),
        };
        if budget.get() == 0 {
            return ran_out(slots, overflow);
        }
        // A line that now starts elsewhere (an earlier line changed) no longer needs its span.
        let kept = needs.len();
        needs.retain(|n| lines.get(n.line).is_some_and(|l| l.0 == n.start));
        if needs.len() != kept {
            continue;
        }
        let mut new = None;
        for (li, (&(start, width), s)) in lines.iter().zip(&slots).enumerate() {
            if width > s.width + 0.5 && !needs.iter().any(|n| n.line == li) && rows.first_fit(s.row, s.span + 1, width, inset).is_some() {
                new = Some(Need { line: li, start, width });
                break;
            }
        }
        if let Some(n) = new {
            needs.push(n);
            continue;
        }
        return ran_out(slots, overflow);
    }
}

/// One pass of [`break_area`]: each line in the next span, skipping spans too narrow for a
/// line in `needs` when a later span holds it.
fn break_area_pass(layout: &mut Layout<RunBrush>, rows: &mut AreaRows<'_>, inset: f32, scale: f32, needs: &[Need]) -> (Vec<Slot>, bool) {
    #[cfg(test)]
    AREA_PASSES.with(|c| c.set(c.get() + 1));
    let mut slots: Vec<Slot> = Vec::new();
    let mut br = layout.break_lines();
    // Per-line widths below are never wider than this, as parley requires.
    br.state_mut().set_layout_max_advance(f32::INFINITY);
    let (mut row, mut span) = (0, 0);
    let mut overflow = false;
    while !br.is_done() {
        let Some((baseline, spans)) = rows.get(row) else {
            overflow = true;
            break;
        };
        let baseline = *baseline;
        let Some(&(x0, x1)) = spans.get(span) else {
            row += 1;
            span = 0;
            continue;
        };
        let width = (x1 - x0 - inset).max(1.0);
        // A line known to be too wide here moves on to the first span that holds it.
        if let Some(w) = needs.iter().find(|n| n.line == slots.len()).map(|n| n.width)
            && w > width + 0.5
            && let Some((r, j)) = rows.first_fit(row, span + 1, w, inset)
        {
            (row, span) = (r, j);
            continue;
        }
        br.state_mut().set_line_max_advance(width / scale);
        if br.break_next().is_none() {
            break;
        }
        // Parley aligns each line within the width it was broken at.
        br.set_prior_line_width(width / scale);
        slots.push(Slot { baseline, x: x0, width, row, span });
        span += 1;
    }
    br.finish();
    (slots, overflow)
}

/// One laid-out line's ascent, descent and leading, as the line loop stacks it. `fallback` is
/// the paragraph's first style, used for a line with no glyph runs.
pub(super) fn line_metrics(
    line: &parley::Line<'_, RunBrush>,
    styles: &[CharStyle],
    ps: &ParagraphStyle,
    k: f32,
    fallback: &CharStyle,
    vertical: bool,
) -> LineMetrics {
    let first_px = fallback.size_pt * k;
    let m = line.metrics();
    // Largest leading among the glyph runs, and half the largest em (a vertical column's half width).
    let mut leading = 0.0f32;
    let mut half = 0.0f32;
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(gr) = item {
            let px = gr.run().font_size();
            half = half.max(px * 0.5);
            let lp = styles.get(gr.style().brush.0 as usize).and_then(|st| st.leading_pt);
            leading = leading.max(lp.map_or(ps.auto_leading * px, |l| l * k));
        }
    }
    if leading == 0.0 {
        leading = fallback.leading_pt.map_or(ps.auto_leading * first_px, |l| l * k);
    }
    let (ascent, descent) = if vertical {
        let h = if half > 0.0 { half } else { first_px * 0.5 };
        (h, h)
    } else if m.ascent > 0.0 || m.descent > 0.0 {
        (m.ascent, m.descent)
    } else {
        (first_px * 0.8, first_px * 0.2)
    };
    LineMetrics { ascent, descent, leading }
}

/// Largest ascent, descent and leading over the lines of a broken paragraph, by
/// [`line_metrics`], so an area paragraph's rows are planned with the metrics it is stacked with.
pub(super) fn area_metrics(layout: &Layout<RunBrush>, styles: &[CharStyle], ps: &ParagraphStyle, k: f32, fallback: &CharStyle, vertical: bool) -> LineMetrics {
    layout.lines().map(|line| line_metrics(&line, styles, ps, k, fallback, vertical)).fold(LineMetrics { ascent: 0.0, descent: 0.0, leading: 0.0 }, |a, b| {
        LineMetrics { ascent: a.ascent.max(b.ascent), descent: a.descent.max(b.descent), leading: a.leading.max(b.leading) }
    })
}
