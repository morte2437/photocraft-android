//! Pure guide-layout calculation shared by the command and its non-destructive UI preview.

use photocraft_doc::Guides;
use photocraft_geom::Rect;
use serde_json::Value;

use crate::prefs::Unit;
use crate::{EngineError, Result, Session};

const COMMAND: &str = "view.newGuideLayout";
const MAX_COUNT: u64 = 1000;
const MAX_EDGES: u64 = 16_000;

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

/// A length as given: a number of pixels, or text with a unit as typed in Photoshop's New Guide
/// Layout fields ("2 cm", "0.25in", "12 pt", "10%"), resolved against each target (#4984).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Length {
    Px(f64),
    In(f64, Unit),
}

impl Length {
    /// Pixels at the document's `dpi`; percent is of `extent` (the target's width for column
    /// sizes and left/right margins, its height for row sizes and top/bottom margins).
    fn px(self, key: &str, dpi: f64, extent: f64, ppi: f64) -> Result<f64> {
        let v = match self {
            Length::Px(v) => v,
            Length::In(v, unit) => unit.to_px(v, dpi, extent, ppi),
        };
        if v.is_finite() && v >= 0.0 { Ok(v) } else { Err(bad(format!("{key} must be a finite non-negative length"))) }
    }
}

fn length_of(key: &str, v: &Value) -> Result<Length> {
    let invalid = || bad(format!("{key} must be a non-negative number of pixels, or text with px, in, cm, mm, pt, pc or %"));
    let length = match (v.as_f64(), v.as_str()) {
        (Some(n), _) => Length::Px(n),
        (None, Some(text)) => match Unit::parse_length(text).ok_or_else(invalid)? {
            (n, None | Some(Unit::Pixels)) => Length::Px(n),
            (n, Some(unit)) => Length::In(n, unit),
        },
        _ => return Err(invalid()),
    };
    match length {
        Length::Px(n) | Length::In(n, _) if n.is_finite() && n >= 0.0 => Ok(length),
        _ => Err(invalid()),
    }
}

fn number(p: &Value, key: &str, default: Length) -> Result<Length> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => length_of(key, v),
    }
}

fn count(p: &Value, key: &str) -> Result<u64> {
    let n = match p.get(key) {
        None => 0,
        Some(v) => v.as_u64().ok_or_else(|| bad(format!("{key} must be a whole number")))?,
    };
    // Each column/row costs a loop iteration plus a duplicate scan, so an
    // absurd count from the caller would block the app for minutes (#704).
    // Photoshop's own dialog caps at 32; 1000 is a generous ceiling that
    // still finishes instantly.
    if n > MAX_COUNT {
        return Err(bad(format!("{key} must be {MAX_COUNT} or fewer (got {n})")));
    }
    Ok(n)
}

fn flag(p: &Value, key: &str) -> Result<bool> {
    match p.get(key) {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("{key} must be a boolean"))),
    }
}

/// Document coordinates of every selected artboard, including boards containing selected layers.
pub fn selected_artboards(s: &Session) -> Vec<Rect> {
    let Some(st) = s.active() else { return Vec::new() };
    let selected = st.selected_layers();
    let ids: std::collections::HashSet<_> = selected.iter().filter_map(|id| st.doc.artboard_of(*id)).collect();
    st.doc.artboards().into_iter().filter(|(id, _, _)| ids.contains(id)).map(|(_, _, board)| board.rect).collect()
}

fn targets(s: &Session, p: &Value) -> Result<Vec<Rect>> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let target = match p.get("target") {
        None => "document",
        Some(v) => v.as_str().ok_or_else(|| bad("target must be a string"))?,
    };
    match target {
        "document" => Ok(vec![st.doc.bounds()]),
        "selectedArtboards" => {
            let boards = selected_artboards(s);
            if boards.is_empty() { Err(bad("select an artboard or a layer inside one")) } else { Ok(boards) }
        }
        _ => Err(bad("target must be document or selectedArtboards")),
    }
}

fn push_unique(out: &mut Vec<f64>, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(bad("guide coordinates exceed the supported range"));
    }
    if !out.iter().any(|x| (*x - value).abs() < 0.01) {
        out.push(value);
    }
    Ok(())
}

/// Guide positions for `count` columns (or rows) across `[start, end]`, Photoshop-style: both
/// edges of every column; a fixed `width` packs columns from `start` (or centres them).
fn axis(start: f64, end: f64, count: u64, fixed: f64, gap: f64, center: bool, out: &mut Vec<f64>) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    let available = end - start;
    let n = count as f64;
    let width = if fixed > 0.0 { fixed } else { (available - (n - 1.0) * gap) / n };
    let total = n * width + (n - 1.0) * gap;
    if width <= 0.0 || !total.is_finite() || total > available + 0.001 {
        return Err(bad("count, size and gutter must fit inside the target margins"));
    }
    let origin = if center && fixed > 0.0 { start + (available - total) / 2.0 } else { start };
    for i in 0..count {
        let a = origin + i as f64 * (width + gap);
        push_unique(out, a)?;
        push_unique(out, a + width)?;
    }
    Ok(())
}

/// New lines and the exact guide set that OK will commit. This never edits the session.
#[derive(Clone, Debug)]
pub struct Layout {
    pub added: Guides,
    pub guides: Guides,
}

pub fn calculate(s: &Session, p: &Value) -> Result<Layout> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let cols = count(p, "columns")?;
    let rows = count(p, "rows")?;
    let zero = Length::Px(0.0);
    let width = number(p, "width", zero)?;
    let height = number(p, "height", zero)?;
    let gap = number(p, "gutter", zero)?;
    let row_gap = number(p, "rowGutter", gap)?;
    let center = flag(p, "centerColumns")?;
    let clear = flag(p, "clearExisting")?;
    let margins = match p.get("margin") {
        Some(Value::Array(a)) => {
            if a.len() != 4 {
                return Err(bad("margin must contain top, left, bottom and right"));
            }
            let mut m = [zero; 4];
            for (out, value) in m.iter_mut().zip(a) {
                *out = length_of("margin", value)?;
            }
            m
        }
        _ => [number(p, "margin", zero)?; 4],
    };
    let dpi = f64::from(st.doc.resolution_dpi);
    let ppi = s.prefs().units_and_rulers.point_size.per_inch();
    let bounds = targets(s, p)?;
    // Bound the aggregate work as well as each count, including multi-artboard requests.
    if (cols + rows + 4).saturating_mul(2).saturating_mul(bounds.len() as u64) > MAX_EDGES {
        return Err(bad("the selected targets would create too many guide lines"));
    }
    let mut added = Guides::default();
    for rect in &bounds {
        let (across, down) = (f64::from(rect.width()), f64::from(rect.height()));
        let [top, left, bottom, right] =
            [(margins[0], down), (margins[1], across), (margins[2], down), (margins[3], across)].map(|(m, extent)| m.px("margin", dpi, extent, ppi));
        let (top, left, bottom, right) = (top?, left?, bottom?, right?);
        let (x0, y0, x1, y1) = (f64::from(rect.x0) + left, f64::from(rect.y0) + top, f64::from(rect.x1) - right, f64::from(rect.y1) - bottom);
        if x0 >= x1 || y0 >= y1 {
            return Err(bad("margins must leave space inside the target"));
        }
        if [top, left, bottom, right].iter().any(|x| *x != 0.0) {
            push_unique(&mut added.vertical, x0)?;
            push_unique(&mut added.vertical, x1)?;
            push_unique(&mut added.horizontal, y0)?;
            push_unique(&mut added.horizontal, y1)?;
        }
        let (width, gap) = (width.px("width", dpi, across, ppi)?, gap.px("gutter", dpi, across, ppi)?);
        let (height, row_gap) = (height.px("height", dpi, down, ppi)?, row_gap.px("rowGutter", dpi, down, ppi)?);
        axis(x0, x1, cols, width, gap, center, &mut added.vertical)?;
        axis(y0, y1, rows, height, row_gap, center, &mut added.horizontal)?;
    }
    if added.vertical.is_empty() && added.horizontal.is_empty() {
        return Err(bad("give \"columns\", \"rows\" or a \"margin\""));
    }
    let mut guides = st.doc.guides.clone();
    if clear {
        if p.get("target").and_then(Value::as_str) == Some("selectedArtboards") {
            // Guides are stored as document-wide axes, matching Clear Selected Artboard Guides.
            guides.vertical.retain(|v| !bounds.iter().any(|r| *v >= f64::from(r.x0) && *v <= f64::from(r.x1)));
            guides.horizontal.retain(|v| !bounds.iter().any(|r| *v >= f64::from(r.y0) && *v <= f64::from(r.y1)));
        } else {
            guides = Guides::default();
        }
        // Cleared guides take their colours along, so a new guide at the same place doesn't
        // inherit one.
        guides.prune_colors();
    }
    for x in &added.vertical {
        push_unique(&mut guides.vertical, *x)?;
    }
    for y in &added.horizontal {
        push_unique(&mut guides.horizontal, *y)?;
    }
    Ok(Layout { added, guides })
}

pub(crate) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let layout = calculate(s, p)?;
    let result = serde_json::json!({"vertical":layout.added.vertical.len(),"horizontal":layout.added.horizontal.len()});
    s.edit("New Guide Layout", |doc, _| {
        doc.guides = layout.guides;
        Ok(())
    })?;
    Ok(result)
}
