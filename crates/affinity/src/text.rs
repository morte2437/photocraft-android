//! Artistic and frame text: the story's characters with their font, size, colour, tracking and
//! leading, the paragraph alignment and the first baseline. Layout (line breaks in frames, kerning,
//! OpenType features) is left to the importing application.

use crate::model::{Affine, Align, Point, Reader, Text, TextRun, rect};
use crate::paint::{self, Paint};
use crate::stream::{ObjId, Stream, Value};

/// Characters read per text node (stories are split into blocks; this caps hostile ones).
const MAX_CHARS: usize = 1 << 20;

pub(crate) fn read(r: &mut Reader, id: ObjId, world: Affine) -> Option<Text> {
    let s = r.s;
    let story = s.obj(id, b"StSt")?;
    let frame = s.obj(id, b"TxtH")?;
    let artistic = s.is(frame, b"ArFr");
    let bounds = s.floats::<4>(frame, b"FrmB").map(rect)?;
    let mut runs = Vec::new();
    let mut align = None;
    let mut total = 0usize;
    let mut placeholders = false;
    for block in s.objs(story, b"Blok") {
        let glyphs = s.obj(block, b"Glyp")?;
        let chars = block_chars(s, glyphs, total, &mut placeholders);
        total = total.checked_add(chars.len())?;
        if total > MAX_CHARS {
            r.warn("text longer than a million characters (truncated)");
            break;
        }
        let mut start = 0usize;
        for run in s.obj(block, b"GAtt").map(|g| s.objs(g, b"Runs")).unwrap_or_default() {
            let end = usize::try_from(s.int(run, b"Indx")?).ok()?.min(chars.len());
            if end <= start {
                continue;
            }
            let text: String = chars.get(start..end)?.iter().filter(|c| **c != '\0' && **c != '\u{FFFC}').collect();
            start = end;
            let Some(attrs) = s.obj(run, b"Item") else { continue };
            if !text.is_empty() {
                runs.push(run_attrs(r, attrs, text, world));
            }
        }
        if align.is_none() {
            align = s.obj(block, b"PAtt").and_then(|p| s.objs(p, b"Runs").first().copied()).and_then(|run| s.obj(run, b"Item")).map(|p| paragraph_align(r, p));
        }
    }
    if placeholders {
        r.warn("text fields such as page numbers (left out of the text)");
    }
    let align = align.unwrap_or(Align::Left);
    let first_baseline = bounds.y0 + s.f64(frame, b"ArtV").unwrap_or(0.0);
    let anchor_x = match align {
        Align::Center => (bounds.x0 + bounds.x1) / 2.0,
        Align::Right => bounds.x1,
        _ => bounds.x0,
    };
    if !artistic {
        if s.objs(frame, b"ColW").len() > 1 || matches!(s.field(frame, b"ColW"), Some(Value::Array(a)) if a.len() > 1) {
            r.warn("text frames with several columns (imported as one column)");
        }
        if s.class(id) == Some(crate::stream::Tag::of(b"TxtC")) {
            r.warn("text frames with a curved outline (imported as rectangular frames)");
        }
    }
    Some(Text { runs, align, anchor: Point { x: anchor_x, y: first_baseline }, frame: if artistic { None } else { Some(bounds) }, transform: world })
}

/// The characters of one text block. A block without a UTF-8 string is a list of segments: each
/// inline object (page number, anchor) takes one `U+FFFC` before its segment's text. Stops once
/// `total` plus these characters would pass the cap.
fn block_chars(s: &Stream, glyphs: ObjId, total: usize, placeholders: &mut bool) -> Vec<char> {
    // One past the allowance is enough to trip the check in `read`; both arms stop there.
    let room = MAX_CHARS.saturating_sub(total).saturating_add(1);
    match s.str(glyphs, b"Utf8") {
        Some(t) => t.chars().take(room).collect(),
        None => {
            // Inline objects (page numbers, anchors) each take one index before the segment text.
            let mut v = Vec::new();
            // Shared segments repeated by reference must not expand beyond the allowance first.
            for seg in s.objs(glyphs, b"Mixd") {
                if v.len() >= room {
                    break;
                }
                let inline = s.objs(seg, b"Glys").len();
                v.extend(std::iter::repeat_n('\u{FFFC}', inline.min(room - v.len())));
                *placeholders |= inline > 0;
                v.extend(s.str(seg, b"Utf8").unwrap_or_default().chars().take(room.saturating_sub(v.len())));
            }
            v
        }
    }
}

fn slot<T: Copy>(v: Option<&Value>, i: usize, f: impl Fn(&Value) -> Option<T>) -> Option<T> {
    match v? {
        Value::Array(a) => a.get(i).and_then(f),
        _ => None,
    }
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => i64::try_from(*u).ok(),
        _ => None,
    }
}

/// Longest font name kept; real PostScript and family names are far shorter.
const MAX_FONT_NAME: usize = 256;

/// A font name for one run. Runs repeat the same shared string, so its copies are charged
/// against a document-wide byte budget; an overlong name or an exhausted budget leaves it empty.
fn font_name(r: &mut Reader, font: Option<ObjId>, tag: &[u8; 4]) -> String {
    let name = font.and_then(|f| r.s.str(f, tag)).unwrap_or_default();
    if name.len() > MAX_FONT_NAME {
        r.warn("font names longer than 256 bytes (left empty)");
        return String::new();
    }
    if !r.charge_font_name(name.len()) {
        r.warn("font names beyond the document metadata budget (left empty)");
        return String::new();
    }
    name.to_string()
}

fn run_attrs(r: &mut Reader, attrs: ObjId, text: String, world: Affine) -> TextRun {
    let s = r.s;
    let doubles = s.field(attrs, b"Doub");
    let ints = s.field(attrs, b"Ints");
    let size = slot(doubles, 0, float).filter(|v| *v > 0.0 && *v < 1e6).unwrap_or(12.0 * r.dpi / 72.0);
    let tracking = slot(doubles, 4, float).unwrap_or(0.0);
    let leading = (slot(ints, 4, int) == Some(1)).then(|| slot(doubles, 13, float)).flatten().filter(|v| *v > 0.0);
    let objects = match s.field(attrs, b"Objs") {
        Some(Value::Array(a)) => a.iter().map(|v| if let Value::Obj(o) = v { *o } else { None }).collect(),
        _ => Vec::new(),
    };
    let fill = objects.first().copied().flatten().and_then(|f| paint::descriptor(r, f, world)).unwrap_or(Paint::None);
    if objects.get(1).copied().flatten().and_then(|f| paint::descriptor(r, f, world)).is_some_and(|p| p != Paint::None) {
        r.warn("outlined (stroked) text");
    }
    if slot(doubles, 10, float).is_some_and(|h| (h - 1.0).abs() > 1e-3) {
        r.warn("horizontally scaled text");
    }
    let font = s.obj(attrs, b"DFnt");
    let postscript = font_name(r, font, b"Post");
    let family = font_name(r, font, b"Famy");
    TextRun {
        text,
        postscript,
        family,
        weight: font.and_then(|f| s.int(f, b"Wegt")).unwrap_or(400),
        italic: font.and_then(|f| s.bool(f, b"Ital")).unwrap_or(false),
        size,
        tracking,
        leading,
        fill,
    }
}

fn paragraph_align(r: &mut Reader, p: ObjId) -> Align {
    match slot(r.s.field(p, b"Ints"), 0, int) {
        Some(1) => Align::Center,
        Some(2) => Align::Right,
        Some(3) => Align::Justify,
        Some(0) | None => Align::Left,
        Some(_) => {
            r.warn("an unknown paragraph alignment (imported as left)");
            Align::Left
        }
    }
}
