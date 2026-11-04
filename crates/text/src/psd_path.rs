//! Type on a path in a PSD (#7962): the path lives only in the type layer's frame in the
//! document's `Txt2` block (EngineData marks such a layer as box text, `ShapeType 1`, with the
//! path's bounds as its `BoxBounds`).
//!
//! Observed layout (Photoshop 2026, `text/path-circle.psd` and `text/path-wave-open.psd` in
//! photocraft-corpus): a frame `<< /0 << /1 << /0 [ … ] >> /2 << /0 2 /2 [ matrix ] … >> >> >>`
//! has the kind `/2 /0` (absent for point text, 1 for a text box, 2 for type on a path) and its
//! outline at `/1 /0` as a flat list of cubic Bézier segments, eight numbers each
//! (`p0 c1 c2 p1`, the next segment starting where the previous one ends, a closed path ending
//! where it started), in the frame's space; `/2 /2` maps the frame's space to the `TySh` text
//! space. On a closed path Photoshop sets the type from the path's last anchor (the circle's
//! glyph 0 sits on its fourth anchor and runs on through the first), on an open path from its
//! first. Neither oracle has a moved start or flipped type, so those are written into the path
//! itself (see [`frame_path`]).

use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Knot, PathOp, Subpath};
use photocraft_geom::{Affine, Point};

use crate::engine_data::Value as E;

/// The `Txt2` frame kind (`/2 /0`) of type on a path.
pub(crate) const PATH_FRAME: i64 = 2;
/// The `Txt2` frame kind (`/2 /0`) of a text box.
const BOX_FRAME: i64 = 1;
/// Most cubic segments read from a frame (a hostile file cannot make this large).
const MAX_SEGMENTS: usize = 4096;
/// How close (px) two points must be to count as the same: Photoshop writes a segment's start as
/// an exact copy of the previous segment's end.
const SAME: f64 = 1e-3;

/// The kind of the frame dictionary `frame` (the entry's `/0`): 0 point text, 1 box, 2 path.
pub(crate) fn frame_kind(frame: &E) -> i64 {
    frame.path(&["2", "0"]).and_then(E::as_i64).unwrap_or(0)
}

/// Every entry of `v` as a finite number, or `None`.
fn numbers(v: &E) -> Option<Vec<f64>> {
    v.as_array()?.iter().map(E::as_f64).collect()
}

fn same(a: Point, b: Point) -> bool {
    (a.x - b.x).hypot(a.y - b.y) <= SAME
}

/// Whether the handles `a` (in) and `b` (out) of `anchor` are collinear on opposite sides.
fn smooth(anchor: Point, a: Point, b: Point) -> bool {
    let (ax, ay, bx, by) = (a.x - anchor.x, a.y - anchor.y, b.x - anchor.x, b.y - anchor.y);
    let (la, lb) = (ax.hypot(ay), bx.hypot(by));
    la > SAME && lb > SAME && (ax * by - ay * bx).abs() <= 1e-3 * la * lb && ax * bx + ay * by < 0.0
}

fn knot(in_ctrl: Point, anchor: Point, out_ctrl: Point) -> Knot {
    Knot { anchor, in_ctrl, out_ctrl, smooth: smooth(anchor, in_ctrl, out_ctrl) }
}

/// The type path of a path frame `frame` (the entry's `/0`), in the `TySh` text space, with the
/// type starting at its first knot. `Err` says why it can't be read (the caller keeps the box or
/// point text EngineData describes).
pub(crate) fn read(frame: &E) -> Result<Subpath, String> {
    let raw = frame.path(&["1", "0"]).ok_or("the frame has no path")?;
    let v = numbers(raw).ok_or("a path coordinate is not a number")?;
    if v.is_empty() || v.len() % 8 != 0 {
        return Err(format!("its path has {} coordinates, not whole curve segments", v.len()));
    }
    if v.len() / 8 > MAX_SEGMENTS {
        return Err(format!("its path has more than {MAX_SEGMENTS} segments"));
    }
    let matrix = match frame.path(&["2", "2"]) {
        None => Affine::IDENTITY,
        Some(m) => {
            let Some([a, b, c, d, e, f]) = numbers(m).and_then(|m| <[f64; 6]>::try_from(m).ok()) else {
                return Err("its frame matrix is not six numbers".into());
            };
            let m = Affine { m: [a, b, c, d, e, f] };
            let det = m.determinant();
            if !det.is_finite() || det.abs() <= 1e-12 {
                return Err("its frame matrix is degenerate".into());
            }
            m
        }
    };
    let pts: Vec<Point> = v.as_chunks::<2>().0.iter().map(|&[x, y]| matrix.apply(Point::new(x, y))).collect();
    if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("its path is out of range".into());
    }
    let segs: Vec<[Point; 4]> = pts.as_chunks::<4>().0.to_vec();
    if segs.windows(2).any(|w| matches!(w, [a, b] if !same(a[3], b[0]))) {
        return Err("its path has more than one piece".into());
    }
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else { return Err("the frame has no path".into()) };
    let (x0, x1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y0, y1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    if x1 - x0 <= SAME && y1 - y0 <= SAME {
        return Err("its path has no length".into());
    }
    let closed = same(last[3], first[0]);
    let mut knots: Vec<Knot> = Vec::with_capacity(segs.len() + 1);
    for (i, s) in segs.iter().enumerate() {
        let prev = match i.checked_sub(1).and_then(|j| segs.get(j)) {
            Some(p) => p[2],
            None if closed => last[2],
            None => s[0],
        };
        knots.push(knot(prev, s[0], s[1]));
    }
    if closed {
        // The type starts on the closing segment: from the last anchor.
        knots.rotate_right(1);
    } else {
        knots.push(knot(last[2], last[3], last[3]));
    }
    Ok(Subpath { closed, knots, op: PathOp::Combine })
}

/// `path` with its knots in the opposite order (the segments run backwards).
fn reversed(path: &Subpath) -> Subpath {
    let mut knots: Vec<Knot> = path.knots.iter().rev().map(|k| Knot { anchor: k.anchor, in_ctrl: k.out_ctrl, out_ctrl: k.in_ctrl, smooth: k.smooth }).collect();
    if path.closed {
        // Keep the first knot first: k0, k(n-1), …, k1.
        knots.rotate_right(1);
    }
    Subpath { closed: path.closed, knots, op: path.op }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn cubic(s: &[Point; 4], t: f64) -> Point {
    let (a, b, c) = (lerp(s[0], s[1], t), lerp(s[1], s[2], t), lerp(s[2], s[3], t));
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}

/// Pieces each segment is measured in for [`starting_at`].
const STEPS: usize = 64;

/// `path` starting `s` px along it (the type origin moved there): an open path loses the part
/// before `s`, a closed one starts at a new smooth knot there. Unchanged for `s` at or before the
/// start, or (open) at or past the end.
fn starting_at(path: &Subpath, s: f64) -> Subpath {
    let segs = path.segments();
    let lengths: Vec<f64> = segs
        .iter()
        .map(|seg| {
            (1..=STEPS)
                .map(|i| (cubic(seg, (i - 1) as f64 / STEPS as f64), cubic(seg, i as f64 / STEPS as f64)))
                .map(|(a, b)| (b.x - a.x).hypot(b.y - a.y))
                .sum()
        })
        .collect();
    let total: f64 = lengths.iter().sum();
    if !s.is_finite() || !total.is_finite() || total <= SAME {
        return path.clone();
    }
    let s = if path.closed { s.rem_euclid(total) } else { s };
    if s <= SAME || s >= total - SAME {
        return path.clone();
    }
    // The segment and parameter at arc length `s` (linear within a measured piece).
    let mut left = s;
    let mut at = None;
    for (i, (seg, len)) in segs.iter().zip(&lengths).enumerate() {
        if left > *len {
            left -= len;
            continue;
        }
        let mut walked = 0.0;
        for k in 1..=STEPS {
            let (t0, t1) = ((k - 1) as f64 / STEPS as f64, k as f64 / STEPS as f64);
            let (a, b) = (cubic(seg, t0), cubic(seg, t1));
            let piece = (b.x - a.x).hypot(b.y - a.y);
            if walked + piece >= left || k == STEPS {
                let f = if piece > 0.0 { ((left - walked) / piece).clamp(0.0, 1.0) } else { 0.0 };
                at = Some((i, t0 + (t1 - t0) * f, *seg));
                break;
            }
            walked += piece;
        }
        break;
    }
    let Some((i, t, seg)) = at else { return path.clone() };
    // de Casteljau: left half p0 a ab m, right half m bc c p3.
    let (a, b, c) = (lerp(seg[0], seg[1], t), lerp(seg[1], seg[2], t), lerp(seg[2], seg[3], t));
    let (ab, bc) = (lerp(a, b, t), lerp(b, c, t));
    let m = lerp(ab, bc, t);
    let n = path.knots.len();
    if n == 0 {
        return path.clone();
    }
    let next = (i + 1) % n;
    let mut knots = Vec::with_capacity(n + 1);
    knots.push(Knot { anchor: m, in_ctrl: if path.closed { ab } else { m }, out_ctrl: bc, smooth: path.closed });
    let count = if path.closed { n } else { n - next };
    for r in 0..count {
        let j = (next + r) % n;
        let Some(mut k) = path.knots.get(j).copied() else { continue };
        if j == next {
            k.in_ctrl = c;
        }
        if path.closed && j == i {
            k.out_ctrl = a;
        }
        knots.push(k);
    }
    Subpath { closed: path.closed, knots, op: path.op }
}

/// The path a frame is written with for `TextShape::Path { path, start, flip }`: flipped type is
/// the path run backwards and a moved start the path from there on (exactly how PhotoCraft lays
/// type on a path out), so Photoshop, which starts at the path's beginning, sets the same type.
pub(crate) fn frame_path(path: &Subpath, start: f32, flip: bool) -> Subpath {
    let path = if flip { reversed(path) } else { path.clone() };
    starting_at(&path, f64::from(start))
}

fn real(v: f64) -> E {
    E::Real(v)
}

/// The segment list (`/1 /0`) Photoshop reads `path` from: on a closed path the type starts at
/// the last anchor, so the first knot goes last.
fn segment_list(path: &Subpath) -> Option<Vec<E>> {
    let mut knots = path.knots.clone();
    if path.closed && !knots.is_empty() {
        knots.rotate_left(1);
    }
    let segs = Subpath { closed: path.closed, knots, op: path.op }.segments();
    let out: Vec<E> = segs.iter().flatten().flat_map(|p| [real(p.x), real(p.y)]).collect();
    (!out.is_empty() && segs.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite())).then_some(out)
}

/// Whether two type paths are the same within [`SAME`] (the smooth flags aside).
fn same_path(a: &Subpath, b: &Subpath) -> bool {
    a.closed == b.closed
        && a.knots.len() == b.knots.len()
        && a.knots.iter().zip(&b.knots).all(|(p, q)| same(p.anchor, q.anchor) && same(p.in_ctrl, q.in_ctrl) && same(p.out_ctrl, q.out_ctrl))
}

/// The point-text frame Photoshop writes (`<< /0 << /2 << /6 [ -1.0 -1.0 ] /11 << /4 -1 /7 false
/// >> >> >> >>`; also what [`crate::psd`] pads the frame list with).
pub(crate) fn point_frame() -> E {
    let kind = E::Dict(vec![("4".into(), E::Int(-1)), ("7".into(), E::Bool(false))]);
    let data = E::Dict(vec![("6".into(), E::Array(vec![real(-1.0), real(-1.0)])), ("11".into(), kind)]);
    E::Dict(vec![("0".into(), E::Dict(vec![("2".into(), data)]))])
}

/// A text box's frame as Photoshop writes it (`text/paragraph-box-left.psd`): the rectangle as
/// four straight segments from its top left, clockwise.
fn box_frame(x: f64, y: f64, w: f64, h: f64) -> E {
    let c = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)];
    let pts: Vec<E> =
        c.windows(2).flat_map(|p| p.iter().flat_map(|&(a, b)| [(a, b), (a, b)]).collect::<Vec<_>>()).flat_map(|(a, b)| [real(a), real(b)]).collect();
    let kind = E::Dict(vec![("4".into(), E::Int(-2)), ("18".into(), real(-2.0)), ("22".into(), real(0.025))]);
    let data = E::Dict(vec![("0".into(), E::Int(BOX_FRAME)), ("6".into(), E::Array(vec![real(-2.0), real(-2.0)])), ("11".into(), kind)]);
    E::Dict(vec![("0".into(), E::Dict(vec![("1".into(), E::Dict(vec![("0".into(), E::Array(pts))])), ("2".into(), data)]))])
}

/// Brings the frame list entry `entry` (`<< /0 frame >>`) of a type layer up to date with its
/// `shape`: type on a path gets a path frame for its path (left byte for byte when it already
/// holds that path, e.g. Photoshop's own after an unedited save), and a layer no longer on a path
/// loses its path frame (else it would reopen on the path).
pub(crate) fn update_frame(entry: &mut E, shape: &TextShape) {
    let frame = entry.get("0");
    let kind = frame.map_or(0, frame_kind);
    match shape {
        TextShape::Path { path, start, flip } => {
            let path = frame_path(path, *start, *flip);
            if kind == PATH_FRAME && frame.and_then(|f| read(f).ok()).is_some_and(|p| same_path(&p, &path)) {
                return;
            }
            let Some(list) = segment_list(&path) else { return };
            // Keep the other keys of a path frame; anything else becomes one.
            let mut data = match frame.and_then(|f| f.get("2")) {
                Some(d @ E::Dict(_)) if kind == PATH_FRAME => d.clone(),
                _ => E::Dict(vec![("6".into(), E::Array(vec![real(-1.0), real(-1.0)]))]),
            };
            data.set("0", E::Int(PATH_FRAME));
            // The path is written in text space.
            data.set("2", E::Array([1.0, 0.0, 0.0, 1.0, 0.0, 0.0].map(real).to_vec()));
            let mut frame = match frame {
                Some(f @ E::Dict(_)) if kind == PATH_FRAME => f.clone(),
                _ => E::dict(),
            };
            frame.set("1", E::Dict(vec![("0".into(), E::Array(list))]));
            frame.set("2", data);
            *entry = E::Dict(vec![("0".into(), frame)]);
        }
        // Leave unreadable path frames unchanged. They were not imported as detached paths.
        _ if kind != PATH_FRAME || frame.is_none_or(|f| read(f).is_err()) => {}
        TextShape::Box { x, y, width, height } => *entry = box_frame(f64::from(*x), f64::from(*y), f64::from(*width), f64::from(*height)),
        TextShape::Point => *entry = point_frame(),
        // Area type saves as a text box over its path's bounds, as its EngineData does.
        TextShape::Area { path } => {
            *entry = match crate::area::path_bounds(path) {
                Some((x0, y0, x1, y1)) => box_frame(x0, y0, x1 - x0, y1 - y0),
                None => point_frame(),
            }
        }
    }
}

#[cfg(test)]
mod tests;
