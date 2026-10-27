use super::*;

/// Byte offset of character `index` (the text length when `index` is past the end).
pub fn byte_index(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(b, _)| b)
}

/// Character index of byte `byte`, floored to a char boundary so a bad offset never panics.
pub fn char_index(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

/// Line containing a byte offset. At a shared wrap boundary the caret belongs to
/// the following line, while an offset at the end of the text stays on the last.
pub fn line_index(layout: &TextLayout, byte: usize) -> usize {
    layout.lines.iter().rposition(|ln| byte >= ln.range.start && byte <= ln.range.end).unwrap_or_else(|| layout.lines.len().saturating_sub(1))
}

/// Nearest caret to a text-space point: character index and the line it sits on.
pub fn hit_char(layout: &TextLayout, text: &str, x: f32, y: f32) -> (usize, usize) {
    let byte = layout.hit_test(x, y);
    // At a soft-wrap boundary either visual line can share this byte offset. The
    // pointer's line, not the caret's preferred (following) line, is authoritative.
    let (_, line_y) = layout.to_line(x, y);
    let line = layout.closest_line(line_y).map_or_else(|| line_index(layout, byte), |(index, _)| index);
    (char_index(text, byte), line)
}

/// Text-space point inside the laid-out line boxes, expanded by `slop` px on every side.
pub fn text_point_inside(layout: &TextLayout, x: f32, y: f32, slop: f32) -> bool {
    let slop = if slop.is_finite() { slop.max(0.0) } else { 0.0 };
    layout.bounds().is_some_and(|b| x >= b[0] - slop && x <= b[2] + slop && y >= b[1] - slop && y <= b[3] + slop)
}

/// Character index of the word boundary before (`forward` is false) or after `idx`.
///
/// A word is a run of alphanumeric characters, the rule the Type tool has always used.
pub fn word_boundary(text: &str, idx: usize, forward: bool) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = idx.min(chars.len());
    if forward {
        while i < chars.len() && !chars[i].is_alphanumeric() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
    } else {
        while i > 0 && !chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
    }
    i
}

/// Caret on the neighbouring line (`dir` < 0 previous, otherwise next), keeping `x`
/// (line space: the position along the line). Past the first or last line the caret
/// goes to the start or end of the text. Line space is the same for both orientations,
/// so a column of vertical type steps the same way a line of horizontal type does.
pub fn line_step(layout: &TextLayout, text: &str, idx: usize, x: f32, dir: i32) -> usize {
    let n = text.chars().count();
    let idx = idx.min(n);
    if layout.lines.is_empty() {
        return idx;
    }
    let current_line = line_index(layout, byte_index(text, idx));
    let target_line = if dir < 0 {
        let Some(target) = current_line.checked_sub(1) else { return 0 };
        target
    } else {
        let Some(target) = current_line.checked_add(1).filter(|target| *target < layout.lines.len()) else { return n };
        target
    };
    char_index(text, layout.hit_test_x_on_line(target_line, x))
}

/// Line start (`end` false) or end for the line containing `idx`, as a character index.
pub fn line_edge(layout: &TextLayout, text: &str, idx: usize, end: bool) -> usize {
    let n = text.chars().count();
    let idx = idx.min(n);
    let byte = byte_index(text, idx);
    let line = layout.lines.get(line_index(layout, byte));
    line.map_or(idx, |ln| char_index(text, if end { ln.range.end } else { ln.range.start }))
}
