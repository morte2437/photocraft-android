use super::*;

/// Photoshop's default tab stops: every half inch, measured from the anchor of point text or the
/// left edge of a paragraph box. PSD type carries no custom tab stops, so these are the only ones.
pub const DEFAULT_TAB_STOP_PT: f32 = 36.0;

/// The first tab stop after `pos` (px from the anchor or box edge); stops are `interval` apart.
pub(super) fn next_tab_stop(pos: f32, interval: f32) -> f32 {
    if interval > 0.0 && pos.is_finite() { ((pos + 1e-3) / interval).floor() * interval + interval } else { pos }
}

/// A tab as line breaking saw it: its shaped advance (before horizontal scale), the horizontal
/// scale, and the width that takes it to its tab stop on its line.
pub(super) struct TabWidth {
    pub(super) advance: f32,
    pub(super) scale: f32,
    pub(super) width: f32,
}

/// The width of each tab (offsets `tabs` in the paragraph text) on the broken lines of `layout`,
/// before kerning and alignment.
pub(super) fn tab_widths(layout: &Layout<RunBrush>, tabs: &[usize], styles: &[CharStyle], indent: f32, first_indent: f32, interval: f32) -> Vec<TabWidth> {
    let mut out: Vec<TabWidth> = tabs.iter().map(|_| TabWidth { advance: interval, scale: 1.0, width: interval }).collect();
    for (li, line) in layout.lines().enumerate() {
        let mut pos = indent + if li == 0 { first_indent } else { 0.0 };
        let mut seen: Vec<usize> = Vec::new();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            let run = gr.run();
            if seen.contains(&run.index()) {
                continue;
            }
            seen.push(run.index());
            let scale = styles.get(gr.style().brush.0 as usize).map_or(1.0, |st| if st.horizontal_scale > 0.0 { st.horizontal_scale } else { 1.0 });
            for c in run.visual_clusters() {
                let advance = c.advance();
                match tabs.binary_search(&c.text_range().start).ok().and_then(|i| out.get_mut(i)) {
                    Some(t) => {
                        let stop = next_tab_stop(pos, interval);
                        *t = TabWidth { advance, scale, width: stop - pos };
                        pos = stop;
                    }
                    None => pos += advance * scale,
                }
            }
        }
    }
    out
}

/// Byte ranges of paragraphs (each including its `\r`, `\n` or `\r\n` terminator). Text ending
/// with a break yields a final empty paragraph, like Photoshop's trailing empty line.
pub fn split_paragraphs(text: &str) -> Vec<Range<usize>> {
    let b = text.as_bytes();
    let mut v = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' => {
                let end = if b.get(i + 1) == Some(&b'\n') { i + 2 } else { i + 1 };
                v.push(start..end);
                start = end;
                i = end;
            }
            b'\n' => {
                v.push(start..i + 1);
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    v.push(start..b.len());
    v
}

pub(super) fn strip_break(text: &str, r: &Range<usize>) -> usize {
    let s = &text[r.clone()];
    r.start + s.trim_end_matches(['\r', '\n']).len()
}

pub(super) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace(['\\', '"'], ""))
}
