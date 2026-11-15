//! Warp Text: bends the glyph outlines of a type layer with one of Photoshop's fifteen warp
//! styles (the PSD `warp` descriptor's `warpStyle`).
//!
//! The style math lives in `photocraft_geom::warp` (shared with Edit › Transform › Warp): our
//! own closed-form approximations of each style's look (observed behaviour, not Adobe's
//! formulas), the identity at `bend = 0` and continuous in `bend`. The mapping is evaluated per
//! point; the renderer subdivides long segments so straight edges bend.

use photocraft_doc::text::{TextShape, TextWarp};
use photocraft_doc::vector::Subpath;
use photocraft_geom::warp::{StyleWarp, WarpStyle};

/// Warp styles in Photoshop's menu order: (PSD `warpStyle` value, short id used by commands).
pub const STYLES: [(&str, &str); 15] = [
    ("warpArc", "arc"),
    ("warpArcLower", "arcLower"),
    ("warpArcUpper", "arcUpper"),
    ("warpArch", "arch"),
    ("warpBulge", "bulge"),
    ("warpShellLower", "shellLower"),
    ("warpShellUpper", "shellUpper"),
    ("warpFlag", "flag"),
    ("warpWave", "wave"),
    ("warpFish", "fish"),
    ("warpRise", "rise"),
    ("warpFisheye", "fisheye"),
    ("warpInflate", "inflate"),
    ("warpSqueeze", "squeeze"),
    ("warpTwist", "twist"),
];

/// PSD style name (`warpArc`) for a short id (`arc`, case-insensitive) or a PSD name.
pub fn psd_style(id: &str) -> Option<&'static str> {
    let l = id.to_ascii_lowercase();
    if l == "none" || l == "warpnone" {
        return Some("warpNone");
    }
    STYLES.iter().find(|(psd, short)| psd.to_ascii_lowercase() == l || short.to_ascii_lowercase() == l).map(|(psd, _)| *psd)
}

/// Short id (`flag`, `none`) for a PSD style name (`warpFlag`, `warpNone`): the reverse of [`psd_style`].
pub fn short_style(psd: &str) -> Option<&'static str> {
    if psd == "warpNone" {
        return Some("none");
    }
    STYLES.iter().find(|(p, _)| *p == psd).map(|(_, short)| *short)
}

/// Where a glyph goes when the layout is bent: see [`Warp::place_glyph`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Not path text: bend the outline point by point with [`Warp::apply`].
    Bend,
    /// Beyond either end of the path: the glyph is not drawn.
    Hidden,
    /// Path text: the glyph keeps its shape and is moved rigidly by this text-space map
    /// (`[a, b, c, d, e, f]`, applied to text-space points).
    Frame([f64; 6]),
}

/// A warp prepared for one layout: the shared style math of
/// [`photocraft_geom::warp::StyleWarp`] (also used by Edit › Transform › Warp) over the text box,
/// or a path the type runs along.
#[derive(Clone, Debug)]
pub struct Warp(Kind);

#[derive(Clone, Debug)]
enum Kind {
    Style(Box<StyleWarp>),
    Path(PathFrame),
}

impl Warp {
    /// Prepares `w` for text whose bounds are `[x0, y0, x1, y1]` (text space). `None` for
    /// `warpNone`, unknown styles, an all-zero warp or empty bounds.
    pub fn new(w: &TextWarp, bounds: [f32; 4]) -> Option<Warp> {
        let style = WarpStyle::parse(&w.style).filter(|s| s.is_preset() && s.psd_name() == w.style)?;
        StyleWarp::new(style, f64::from(w.value), f64::from(w.horizontal_distortion), f64::from(w.vertical_distortion), !w.horizontal, bounds.map(f64::from))
            .map(|s| Warp(Kind::Style(Box::new(s))))
    }

    /// The deformation of type on a path (`None` for any other shape, or a path without length).
    pub fn on_path(shape: &TextShape) -> Option<Warp> {
        let TextShape::Path { path, start, end, flip, .. } = shape else { return None };
        PathFrame::new(path, *start, *end, *flip).map(|f| Warp(Kind::Path(f)))
    }

    /// Path text with its logical left edge after paragraph alignment. A closed path has
    /// one lap of capacity from this edge, including centred and right-aligned text whose
    /// coordinates start below zero. Open paths keep their existing endpoint clipping.
    pub fn on_path_with_text_start(shape: &TextShape, text_start: f64) -> Option<Warp> {
        Self::on_path_oriented(shape, text_start, false)
    }

    /// Path text with its logical line start and orientation.
    /// Deforms text along a path with explicit line origin and vertical orientation support.
    pub fn on_path_oriented(shape: &TextShape, text_start: f64, vertical: bool) -> Option<Warp> {
        if !text_start.is_finite() {
            return None;
        }
        let mut warp = Self::on_path(shape)?;
        if let Kind::Path(frame) = &mut warp.0 {
            frame.text_start = text_start;
            frame.vertical = vertical;
        }
        Some(warp)
    }

    /// Text-space x nearest a point on a type path, using the same seam and one-lap
    /// interval as glyph placement. Used when clicking to place the caret after the seam.
    pub fn path_text_x(&self, x: f64, y: f64) -> Option<f64> {
        let Kind::Path(frame) = &self.0 else { return None };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let offset = frame.project(x, y) - frame.start;
        Some(if frame.closed {
            let origin = if frame.range_length.is_some() { 0.0 } else { frame.text_start };
            (offset - origin).rem_euclid(frame.length()) + origin
        } else {
            offset
        })
    }

    /// Caret hit testing in the occupied text interval. In a closed path's unused gap,
    /// choose the nearer endpoint along the path instead of always choosing the last caret.
    pub fn path_text_x_near_text(&self, x: f64, y: f64, text_end: f64) -> Option<f64> {
        let offset = self.path_text_x(x, y)?;
        let Kind::Path(frame) = &self.0 else { return None };
        if let Some(span) = frame.range_length {
            let lower = if frame.closed { 0.0 } else { (-frame.start).max(0.0) };
            let upper = if frame.closed { span.min(frame.length()) } else { span.min(frame.length() - frame.start) };
            let lower = lower.max(frame.text_start);
            let upper = upper.min(text_end).max(lower);
            let mut nearest = offset;
            if frame.closed && offset > upper && lower + frame.length() - offset < offset - upper {
                nearest -= frame.length();
            } else if frame.closed && offset < lower && offset + frame.length() - upper < lower - offset {
                nearest += frame.length();
            }
            return Some(nearest.clamp(lower, upper));
        }
        if frame.closed && text_end.is_finite() && offset > text_end {
            let previous = offset - frame.length();
            if frame.text_start - previous < offset - text_end {
                return Some(previous);
            }
        }
        Some(offset)
    }

    /// Clip a decoration interval to an explicit path range.
    pub(crate) fn clip_decoration(&self, x0: f64, x1: f64) -> Option<(f64, f64)> {
        let Kind::Path(frame) = &self.0 else { return Some((x0, x1)) };
        let Some(span) = frame.range_length else { return Some((x0, x1)) };
        let lower = if frame.closed { 0.0 } else { (-frame.start).max(0.0) };
        let upper = if frame.closed { span.min(frame.length()) } else { span.min(frame.length() - frame.start) };
        let (a, b) = (x0.max(lower), x1.min(upper));
        (a < b).then_some((a, b))
    }

    /// Longest segment (text-space px) worth sending through [`Warp::apply`] unsplit.
    pub fn max_segment(&self) -> f64 {
        match &self.0 {
            Kind::Style(s) => s.max_segment(),
            Kind::Path(_) => 2.0,
        }
    }

    /// Maps a text-space point.
    /// Applies path deformation to map horizontal or vertical text coordinates onto the curved baseline.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        match &self.0 {
            Kind::Style(s) => s.apply(x, y),
            Kind::Path(p) => p.apply(x, y),
        }
    }

    /// How to place a glyph whose horizontal centre is at text-space `cx`.
    pub fn place_glyph(&self, cx: f64) -> Placement {
        match &self.0 {
            Kind::Style(_) => Placement::Bend,
            Kind::Path(p) => p.frame(cx).map_or(Placement::Hidden, Placement::Frame),
        }
    }
}

/// The arc length along `path`, measured as type on it measures it (from the end it starts at
/// with `flip`), of the point on the path nearest (`x`, `y`): the `start` that puts the type's
/// origin where the Type tool clicked on the path. `None` for a path without length.
pub fn arc_length_at(path: &Subpath, flip: bool, x: f64, y: f64) -> Option<f64> {
    PathFrame::new(path, 0.0, None, flip).map(|f| f.project(x, y))
}

/// Effective explicit range in text-space units, limited by the path capacity.
pub fn path_range_length(shape: &TextShape) -> Option<f64> {
    let TextShape::Path { path, start, end: Some(end), flip } = shape else { return None };
    let frame = PathFrame::new(path, *start, Some(*end), *flip)?;
    let capacity = if frame.closed { frame.length() } else { (frame.length() - frame.start).max(0.0) };
    Some(frame.range_length?.min(capacity))
}

/// Most cubic segments of a type path that are followed (a hostile file cannot make this large).
const MAX_PATH_SEGMENTS: usize = 4096;

/// Geometry for sampling and projecting points onto a text path.
#[derive(Clone, Debug)]
pub struct PathGeometry {
    frame: PathFrame,
}

impl PathGeometry {
    /// Flatten a non-degenerate path, optionally reversing its direction.
    pub fn new(path: &Subpath, flip: bool) -> Option<Self> {
        Some(Self { frame: PathFrame::new(path, 0.0, None, flip)? })
    }

    /// Total path length.
    /// Returns the total arc length of the flattened path frame.
    pub fn length(&self) -> f64 {
        self.frame.length()
    }

    /// Largest `f32` arc-length that never exceeds `length()`.
    pub fn canonical_length_f32(&self) -> f32 {
        let length = self.length();
        let f = length as f32;
        if f64::from(f) > length && f > 0.0 { f32::from_bits(f.to_bits() - 1) } else { f }
    }

    /// Returns the geometry traversed in the opposite direction.
    pub fn flipped(&self) -> Self {
        let mut pts = self.frame.pts.clone();
        pts.reverse();
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if let Some(q) = i.checked_sub(1).and_then(|j| pts.get(j)) {
                acc += (p.0 - q.0).hypot(p.1 - q.1);
            }
            cum.push(acc);
        }
        Self {
            frame: PathFrame {
                pts,
                cum,
                closed: self.frame.closed,
                start: self.frame.start,
                range_length: self.frame.range_length,
                text_start: self.frame.text_start,
                vertical: self.frame.vertical,
            },
        }
    }

    /// Sample a point and unit tangent at an arc length.
    pub fn sample(&self, arc: f64) -> Option<((f64, f64), (f64, f64))> {
        if !arc.is_finite() {
            return None;
        }
        let (p, t) = self.frame.at(arc);
        (p.0.is_finite() && p.1.is_finite() && t.0.is_finite() && t.1.is_finite()).then_some((p, t))
    }

    /// Projects to candidate branches maintaining movement-aware continuity near `previous`.
    pub fn project_near(&self, x: f64, y: f64, previous: f64) -> Option<(f64, f64)> {
        if !x.is_finite() || !y.is_finite() || !previous.is_finite() {
            return None;
        }
        let arc = self.frame.project_near(x, y, previous);
        let (p, t) = self.sample(arc)?;
        let distance = (x - p.0) * -t.1 + (y - p.1) * t.0;
        distance.is_finite().then_some((arc, distance))
    }

    /// Project a point to arc length and signed distance on the left normal.
    pub fn project(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let arc = self.frame.project(x, y);
        let (p, t) = self.sample(arc)?;
        let distance = (x - p.0) * -t.1 + (y - p.1) * t.0;
        distance.is_finite().then_some((arc, distance))
    }
}

/// A flattened path with arc lengths: text-space `x` is arc length from `start`, `y` the
/// distance to the left of the direction of travel (the side the glyph tops point to).
#[derive(Clone, Debug)]
struct PathFrame {
    pts: Vec<(f64, f64)>,
    /// Arc length at each point.
    cum: Vec<f64>,
    start: f64,
    closed: bool,
    text_start: f64,
    range_length: Option<f64>,
    vertical: bool,
}

impl PathFrame {
    /// Flattens a cubic Bézier subpath into polyline approximations with cumulative arc lengths and optional range limits.
    fn new(path: &Subpath, start: f32, end: Option<f32>, flip: bool) -> Option<PathFrame> {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for seg in path.segments().iter().take(MAX_PATH_SEGMENTS) {
            let p = seg.map(|q| (q.x, q.y));
            if p.iter().any(|q| !q.0.is_finite() || !q.1.is_finite()) {
                return None;
            }
            let poly = (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1) + (p[2].0 - p[1].0).hypot(p[2].1 - p[1].1) + (p[3].0 - p[2].0).hypot(p[3].1 - p[2].1);
            let n = ((poly / 2.0).ceil() as usize).clamp(4, 128);
            if pts.is_empty() {
                pts.push(p[0]);
            }
            for i in 1..=n {
                let t = i as f64 / n as f64;
                let u = 1.0 - t;
                let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                pts.push((a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0, a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1));
            }
        }
        // Drop repeated points so every piece has a direction.
        pts.dedup_by(|b, a| (b.0 - a.0).abs() < 1e-9 && (b.1 - a.1).abs() < 1e-9);
        if flip {
            pts.reverse();
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if let Some(q) = i.checked_sub(1).and_then(|j| pts.get(j)) {
                acc += (p.0 - q.0).hypot(p.1 - q.1);
            }
            cum.push(acc);
        }
        (pts.len() >= 2 && acc.is_finite() && acc > 1e-6 && start.is_finite()).then(|| PathFrame {
            pts,
            cum,
            // A click at the seam may project to either 0 or length (then round as f32).
            start: if path.closed { f64::from(start).rem_euclid(acc) } else { f64::from(start) },
            closed: path.closed,
            text_start: 0.0,
            range_length: end.map(|end| if end.is_finite() && end >= start { f64::from(end) - f64::from(start) } else { 0.0 }),
            vertical: false,
        })
    }

    /// Total arc length of the flattened frame polyline.
    fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Distance and arc length of the point on the flattened path nearest (`x`, `y`).
    fn project_with_dist(&self, x: f64, y: f64) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0.0);
        for (i, w) in self.pts.windows(2).enumerate() {
            let [a, b] = [w[0], w[1]];
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let d = (a.0 + dx * t - x).hypot(a.1 + dy * t - y);
            if d < best.0 {
                best = (d, self.cum.get(i).copied().unwrap_or(0.0) + len2.sqrt() * t);
            }
        }
        best
    }

    /// Arc length of the point on the flattened path nearest (`x`, `y`).
    fn project(&self, x: f64, y: f64) -> f64 {
        self.project_with_dist(x, y).1
    }

    /// Projects a point to candidate branches, maintaining continuity near `previous` while tracking closest projections.
    fn project_near(&self, x: f64, y: f64, previous: f64) -> f64 {
        if self.pts.len() < 2 {
            return 0.0;
        }
        let (min_d, nearest) = self.project_with_dist(x, y);
        let length = self.length();
        // Distance from current pointer (x, y) to the previous projected path point (self.at(previous)).
        // Smoothly broadens the candidate basin limit as the pointer moves farther from the path projection.
        let (prev_pt, _) = self.at(previous);
        let pointer_to_prev_path_dist = (x - prev_pt.0).hypot(y - prev_pt.1);
        let limit = min_d + (pointer_to_prev_path_dist * 0.5 + 0.5).clamp(0.5, 4.0);

        // Adjacent near-distance segments belong to one branch basin. Keep its closest
        // projection before comparing arc continuity between separate branches.
        struct Branch {
            first: usize,
            last: usize,
            distance: f64,
            arc: f64,
        }
        let mut branches: Vec<Branch> = Vec::new();
        for (i, w) in self.pts.windows(2).enumerate() {
            let [a, b] = [w[0], w[1]];
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let distance = (a.0 + dx * t - x).hypot(a.1 + dy * t - y);
            if distance > limit {
                continue;
            }
            let arc = self.cum.get(i).copied().unwrap_or(0.0) + len2.sqrt() * t;
            if let Some(last) = branches.last_mut()
                && last.last + 1 == i
            {
                last.last = i;
                if distance < last.distance {
                    last.distance = distance;
                    last.arc = arc;
                }
            } else {
                branches.push(Branch { first: i, last: i, distance, arc });
            }
        }

        // Seam first and last basins merge across closed path endpoints.
        if self.closed
            && branches.len() > 1
            && branches.first().is_some_and(|f| f.first == 0)
            && branches.last().is_some_and(|l| l.last + 2 == self.pts.len())
            && let Some(last) = branches.pop()
            && let Some(first) = branches.first_mut()
            && last.distance < first.distance
        {
            first.distance = last.distance;
            first.arc = last.arc;
        }

        let continuity_distance = |arc: f64| -> f64 {
            let diff = (arc - previous).abs();
            if self.closed && length > 0.0 {
                let d = diff.rem_euclid(length);
                d.min(length - d)
            } else {
                diff
            }
        };

        let local_threshold = (pointer_to_prev_path_dist * 2.0 + 2.0).clamp(4.0, 24.0);
        branches
            .iter()
            .filter(|b| continuity_distance(b.arc) <= local_threshold)
            .min_by(|a, b| a.distance.total_cmp(&b.distance).then_with(|| continuity_distance(a.arc).total_cmp(&continuity_distance(b.arc))))
            .map(|b| b.arc)
            .unwrap_or_else(|| branches.iter().min_by(|a, b| continuity_distance(a.arc).total_cmp(&continuity_distance(b.arc))).map_or(nearest, |b| b.arc))
    }

    /// Point and unit tangent: closed paths wrap; open paths extend their end tangents.
    fn at(&self, s: f64) -> ((f64, f64), (f64, f64)) {
        let s = if self.closed { s.rem_euclid(self.length()) } else { s };
        let last = self.pts.len() - 1;
        let i = self.cum.partition_point(|&c| c < s).clamp(1, last);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let len = (b.0 - a.0).hypot(b.1 - a.1).max(1e-12);
        let t = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let along = s - self.cum[i - 1];
        ((a.0 + t.0 * along, a.1 + t.1 * along), t)
    }

    /// Maps horizontal or vertical line-space coordinates onto the deformed path baseline.
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        if self.vertical {
            let (p, t) = self.at(self.start + y);
            (p.0 + t.1 * x, p.1 - t.0 * x)
        } else {
            let (p, t) = self.at(self.start + x);
            (p.0 - t.1 * y, p.1 + t.0 * y)
        }
    }

    /// The rigid map for a glyph centred at text-space `cx`; `None` outside the path.
    fn frame(&self, cx: f64) -> Option<[f64; 6]> {
        let s = self.start + cx;
        if !cx.is_finite() {
            return None;
        }
        if let Some(span) = self.range_length {
            if span <= 0.0 || cx < 0.0 || cx > span || (self.closed && cx >= self.length()) || (!self.closed && !(0.0..=self.length()).contains(&s)) {
                return None;
            }
        } else if self.closed {
            if !(self.text_start..self.text_start + self.length()).contains(&cx) {
                return None;
            }
        } else if !(0.0..=self.length()).contains(&s) {
            return None;
        }
        let (p, t) = self.at(s);
        let m = [t.0, t.1, -t.1, t.0, p.0 - t.0 * cx, p.1 - t.1 * cx];
        if self.vertical {
            // Compose the rigid placement frame with the inverse rotation
            // mapping text space (x, y) to canonical line space (y, -x).
            Some([t.1, -t.0, t.0, t.1, m[4], m[5]])
        } else {
            Some(m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies sampling and perpendicular projection along linear and flipped path geometries, rejecting non-finite coordinates.
    #[test]
    fn path_geometry_samples_and_projects() {
        let line = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
        let geometry = PathGeometry::new(&line, false).unwrap();
        assert!((geometry.length() - 100.0).abs() < 1e-9);
        assert_eq!(geometry.sample(25.0), Some(((25.0, 0.0), (1.0, 0.0))));
        assert_eq!(geometry.project(25.0, 3.0), Some((25.0, 3.0)));
        assert_eq!(geometry.project(25.0, -3.0), Some((25.0, -3.0)));
        let flipped = PathGeometry::new(&line, true).unwrap();
        assert_eq!(flipped.sample(25.0), Some(((75.0, 0.0), (-1.0, 0.0))));
        assert_eq!(flipped.project(25.0, 3.0), Some((75.0, -3.0)));
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(geometry.sample(value).is_none());
            assert!(geometry.project(value, 0.0).is_none());
            assert!(geometry.project(0.0, value).is_none());
        }
        assert!(PathGeometry::new(&Subpath::polyline(&[(1.0, 1.0), (1.0, 1.0)]), false).is_none());
    }

    /// Proves dragging across self-intersecting path crossings preserves branch continuity based on prior cursor arc length.
    #[test]
    fn path_geometry_project_near_crossings_stay_on_branch() {
        // Hourglass / bowtie shape with diagonal crossing at (50.0, 50.0).
        let path = Subpath::polygon(&[(0.0, 0.0), (100.0, 100.0), (0.0, 100.0), (100.0, 0.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        // Diagonal 1 from (0, 0) to (100, 100) has arc ~70.71 at (50, 50).
        // Diagonal 2 from (0, 100) to (100, 0) has arc ~312.13 at (50, 50).
        let d1 = 50.0 * 2.0_f64.sqrt();
        let d2 = 100.0 * 2.0_f64.sqrt() + 100.0 + 50.0 * 2.0_f64.sqrt();

        // Approaching crossing along branch 1: stays on branch 1 even if perturbed slightly toward branch 2.
        let near_b1 = geometry.project_near(50.005, 49.995, d1 - 1.0).unwrap();
        assert!((near_b1.0 - d1).abs() < 0.1, "should stay on branch 1: {}", near_b1.0);

        // Approaching crossing along branch 2: stays on branch 2 even if perturbed slightly toward branch 1.
        let near_b2 = geometry.project_near(50.005, 50.005, d2 - 1.0).unwrap();
        assert!((near_b2.0 - d2).abs() < 0.1, "should stay on branch 2: {}", near_b2.0);

        // 0.1 pointer move across crossing from (49.95, 49.95) to (50.05, 49.95) avoids ~241 arc jump.
        let step1 = geometry.project_near(49.95, 49.95, d1 - 1.0).unwrap();
        let step2 = geometry.project_near(50.05, 49.95, step1.0).unwrap();
        assert!((step2.0 - d1).abs() < 0.2, "bowtie 0.1 move stays on branch 1 without ~241 jump: {}", step2.0);

        // Moving deliberately away along branch 2 eventually allows switching to branch 2.
        let away = geometry.project_near(70.0, 30.0, step1.0).unwrap();
        assert!((away.0 - (d2 + 20.0 * 2.0_f64.sqrt())).abs() < 1.0, "deliberate move switches to branch 2: {}", away.0);
    }

    /// Verifies movement-aware continuity preserves branch basins across perturbed non-symmetric crossings.
    #[test]
    fn path_geometry_project_near_perturbed_crossing() {
        // Perturbed bowtie crossing near (50.1, 49.9).
        let path = Subpath::polygon(&[(0.0, 0.0), (100.2, 99.8), (0.0, 99.8), (100.2, 0.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        let (d1, _) = geometry.project(50.1, 49.9).unwrap();
        let step1 = geometry.project_near(49.9, 49.8, d1 - 1.0).unwrap();
        let step2 = geometry.project_near(50.3, 50.0, step1.0).unwrap();
        assert!((step2.0 - d1).abs() < 1.0, "perturbed crossing preserves branch 1: {}", step2.0);
    }

    #[test]
    fn path_geometry_project_near_seam_basins_merge_and_compete() {
        let mut path = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)]);
        path.closed = true;
        let geometry = PathGeometry::new(&path, false).unwrap();
        let len = geometry.length();

        let (arc_first, dist_first) = geometry.project_near(2.0, 0.5, len - 1.0).unwrap();
        assert!((arc_first - 2.0).abs() < 0.1);
        assert!((dist_first - 0.5).abs() < 0.1);

        let (arc_last, _) = geometry.project_near(0.5, 1.0, 1.0).unwrap();
        assert!(arc_last > len - 2.0);

        let mut p = Subpath::polygon(&[(0.0, 0.0), (50.0, 0.0), (0.0, 1.0), (0.0, 50.0)]);
        p.closed = true;
        let g = PathGeometry::new(&p, false).unwrap();
        let mid_arc = g.project(0.0, 1.0).unwrap().0;
        let (arc_mid, _) = g.project_near(0.0, 1.05, mid_arc).unwrap();
        assert!((arc_mid - mid_arc).abs() < 1.0);
    }

    /// Verifies projection tracking remains continuous across sharp polygon corners, closed seams, and perpendicular flips.
    #[test]
    fn path_geometry_project_near_corners_seams_and_flips() {
        // Rectangle: (0, 0) -> (100, 0) -> (100, 100) -> (0, 100) -> (0, 0). Length = 400.
        let path = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();

        // Corner at (100.0, 0.0), arc = 100.0.
        // Dragging around corner from top edge onto right edge.
        let (arc, dist) = geometry.project_near(100.0, 5.0, 99.0).unwrap();
        assert!((arc - 105.0).abs() < 1e-6);
        assert!(dist.abs() < 1e-6);

        // Dragging backwards around corner onto top edge.
        let (arc, dist) = geometry.project_near(95.0, 0.0, 101.0).unwrap();
        assert!((arc - 95.0).abs() < 1e-6);
        assert!(dist.abs() < 1e-6);

        // Closed seam at (0.0, 0.0): dragging forward past the seam.
        let (arc, dist) = geometry.project_near(2.0, 0.0, 399.0).unwrap();
        assert!((arc - 2.0).abs() < 1e-6);
        assert!(dist.abs() < 1e-6);

        // Closed seam: dragging backward before the seam.
        let (arc, dist) = geometry.project_near(0.0, 2.0, 1.0).unwrap();
        assert!((arc - 398.0).abs() < 1e-6);
        assert!(dist.abs() < 1e-6);

        // Flips: dragging perpendicular to curve (off-path) preserves arc length.
        let (arc_above, dist_above) = geometry.project_near(50.0, -10.0, 50.0).unwrap();
        assert!((arc_above - 50.0).abs() < 1e-6);
        assert!((dist_above - -10.0).abs() < 1e-6);

        let (arc_below, dist_below) = geometry.project_near(50.0, 10.0, 50.0).unwrap();
        assert!((arc_below - 50.0).abs() < 1e-6);
        assert!((dist_below - 10.0).abs() < 1e-6);
    }

    /// Ensures off-path cursor drag gestures do not lag or stall across flattened segment boundaries on the same branch.
    #[test]
    fn path_geometry_project_near_off_path_drag_does_not_lag_on_segment_boundaries() {
        let path = Subpath::polyline(&[(0.0, 0.0), (160.0, 0.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        // Dragging horizontally at y = -10 (off-path), previous at 20.0, cursor at x = 30.0.
        // Flattened segment endpoints within 0.01 tolerance on the same branch must not cause lag.
        let (arc, dist) = geometry.project_near(30.0, -10.0, 20.0).unwrap();
        assert!((arc - 30.0).abs() < 1e-6);
        assert!((dist - -10.0).abs() < 1e-6);
    }

    /// Proves sampling coordinates wrap modularly across the seam of a closed polygon path.
    #[test]
    fn path_geometry_closed_wraps() {
        let path = Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        assert_eq!(geometry.sample(2.0), geometry.sample(geometry.length() + 2.0));
        assert_eq!(geometry.sample(-2.0), geometry.sample(geometry.length() - 2.0));
    }

    /// Test helper building a warp for style presets.
    fn warp(style: &str, value: f32) -> Warp {
        let w = TextWarp { style: psd_style(style).unwrap().into(), value, horizontal_distortion: 0.0, vertical_distortion: 0.0, horizontal: true };
        Warp::new(&w, [0.0, -40.0, 200.0, 10.0]).unwrap()
    }

    /// Verifies text decoration clipping truncates at closed path seams and prevents overflow past a single lap.
    #[test]
    fn decoration_ranges_clip_at_closed_seam_and_one_lap() {
        let path = Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        for flip in [false, true] {
            let warp = Warp(Kind::Path(PathFrame::new(&path, 35.0, Some(45.0), flip).unwrap()));
            assert_eq!(warp.clip_decoration(-2.0, 20.0), Some((0.0, 10.0)));
            assert_eq!(warp.clip_decoration(11.0, 20.0), None);
            let lap = Warp(Kind::Path(PathFrame::new(&path, 35.0, Some(135.0), flip).unwrap()));
            let (a, b) = lap.clip_decoration(0.0, 100.0).unwrap();
            assert_eq!(a, 0.0);
            assert!((b - 40.0).abs() < 1e-6, "b={b}");
            let legacy = Warp(Kind::Path(PathFrame::new(&path, 35.0, None, flip).unwrap()));
            assert_eq!(legacy.clip_decoration(-2.0, 100.0), Some((-2.0, 100.0)));
        }
    }

    /// Proves glyph frames are hidden when their centers fall outside the explicit start and end bounds of a path frame.
    #[test]
    fn explicit_path_ranges_clip_glyphs() {
        let line = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
        for flip in [false, true] {
            let frame = PathFrame::new(&line, 20.0, Some(40.0), flip).unwrap();
            assert!(frame.frame(20.0).is_some());
            assert!(frame.frame(21.0).is_none());
            assert!(frame.frame(-1.0).is_none());
            assert!(PathFrame::new(&line, 20.0, Some(20.0), flip).unwrap().frame(0.0).is_none());
            assert!(PathFrame::new(&line, 20.0, Some(f32::NAN), flip).unwrap().frame(0.0).is_none());
            assert!(PathFrame::new(&line, 90.0, Some(120.0), flip).unwrap().frame(11.0).is_none());
            assert!(PathFrame::new(&line, 20.0, None, flip).unwrap().frame(30.0).is_some());
        }
        let mut square = Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        square.closed = true;
        let frame = PathFrame::new(&square, 45.0, Some(50.0), false).unwrap();
        assert!(frame.frame(5.0).is_some());
        assert!(frame.frame(6.0).is_none());
        let frame = PathFrame::new(&square, 0.0, Some(100.0), false).unwrap();
        assert!(frame.frame(frame.length()).is_none());
        let legacy = PathFrame::new(&square, 0.0, None, false).unwrap();
        assert!(legacy.frame(-1.0).is_none());
        assert!(legacy.frame(legacy.length()).is_none());
    }

    /// #218: the Warp Text dialog shows a layer's PSD style by its short id.
    #[test]
    fn short_ids_round_trip_with_psd_styles() {
        assert_eq!(short_style("warpFlag"), Some("flag"));
        assert_eq!(short_style("warpNone"), Some("none"));
        assert_eq!(short_style("warpCustom"), None);
        for (psd, short) in STYLES {
            assert_eq!(short_style(psd), Some(short));
            assert_eq!(psd_style(short_style(psd).unwrap()), Some(psd));
        }
    }

    /// The Type tool's click on a path: the nearest point's arc length, from the other end with flip.
    #[test]
    fn arc_length_at_projects_onto_the_path() {
        let line = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)]);
        let at = |flip, x, y| arc_length_at(&line, flip, x, y).unwrap();
        assert!((at(false, 30.0, -8.0) - 30.0).abs() < 1e-6);
        assert!((at(false, 110.0, 20.0) - 120.0).abs() < 1e-6, "on the second segment");
        assert!((at(true, 30.0, -8.0) - 120.0).abs() < 1e-6, "flipped: measured from the far end");
        assert!(at(false, -50.0, 0.0).abs() < 1e-6, "clamped to the start");
        assert!(arc_length_at(&Subpath::polyline(&[(5.0, 5.0), (5.0, 5.0)]), false, 0.0, 0.0).is_none());
    }

    #[test]
    fn styles_resolve_and_none_is_identity() {
        assert_eq!(psd_style("Arc"), Some("warpArc"));
        assert_eq!(psd_style("warpTwist"), Some("warpTwist"));
        assert_eq!(psd_style("none"), Some("warpNone"));
        assert_eq!(psd_style("spiral"), None);
        let none = TextWarp { style: "warpNone".into(), value: 50.0, ..Default::default() };
        assert!(Warp::new(&none, [0.0, 0.0, 10.0, 10.0]).is_none());
        let flat = TextWarp { style: "warpArc".into(), value: 0.0, ..Default::default() };
        assert!(Warp::new(&flat, [0.0, 0.0, 10.0, 10.0]).is_none());
    }

    #[test]
    fn every_style_is_continuous_and_fixes_the_centre_column() {
        for (_, id) in STYLES {
            let w = warp(id, 50.0);
            let (x, y) = w.apply(100.0, -15.0);
            assert!(x.is_finite() && y.is_finite(), "{id}");
            // Tiny bends barely move anything.
            let t = warp(id, 0.5);
            let (x2, y2) = t.apply(20.0, 0.0);
            assert!((x2 - 20.0).abs() < 2.0 && y2.abs() < 2.0, "{id}: {x2},{y2}");
        }
    }

    #[test]
    fn arc_raises_the_middle_relative_to_the_ends() {
        let w = warp("arc", 50.0);
        let mid = w.apply(100.0, -15.0);
        let end = w.apply(0.0, -15.0);
        // As in Photoshop: the bottom edge keeps its corners, the middle rises and the ends of
        // the upper rows swing out and down along the arc.
        let corner = w.apply(0.0, 10.0);
        assert!(corner.0.abs() < 1e-9 && (corner.1 - 10.0).abs() < 1e-9, "{corner:?}");
        assert!(mid.1 < -15.0, "the middle rises: {mid:?}");
        assert!(end.1 > mid.1 + 5.0, "ends drop below the centre: {end:?}");
        assert!(end.0 < 0.0, "ends swing out along the arc: {end:?}");
        // A negative bend smiles.
        let s = warp("arc", -50.0);
        assert!(s.apply(0.0, -15.0).1 < -20.0);
    }

    #[test]
    fn bulge_and_squeeze_change_height_in_the_middle() {
        let b = warp("bulge", 50.0);
        assert!(b.apply(100.0, -40.0).1 < -50.0);
        // Squeeze (Photoshop) pinches the sides in while the top and bottom bow out a little.
        let s = warp("squeeze", 50.0);
        assert!(s.apply(0.0, -15.0).0 > 10.0, "{:?}", s.apply(0.0, -15.0));
        assert!(s.apply(100.0, -40.0).1 < -40.0);
    }
}

#[cfg(test)]
#[path = "closed_path_tests.rs"]
mod closed_path_tests;
