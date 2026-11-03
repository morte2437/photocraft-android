//! A response curve: how a 0..1 input (pen pressure, tilt, wheel, a fade ramp) maps to a 0..1
//! output. Preferences › Tools › Pressure Curve shapes the pen's pressure globally; every brush
//! dynamic carries its own curve for its control ([`crate::Dynamic::curve`]).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Most control points a curve keeps.
const MAX: usize = 16;

/// A monotone cubic (Fritsch–Carlson) through `[input, output]` control points, so a stronger
/// input never gives less output. Points are sanitised, not trusted: non-finite points are dropped,
/// the rest clamped to 0..1, sorted by input (one point per input, the later output wins), capped
/// at [`Self::MAX_POINTS`], and their outputs made non-decreasing. Before the first point and after
/// the last the curve stays flat. Fewer than two usable points is linear.
///
/// Fixed-size and `Copy`, so a brush evaluates it per dab without allocating. In JSON it is its
/// list of points, `[[0,0],[0.5,0.2],[1,1]]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponseCurve {
    n: u8,
    xs: [f32; MAX],
    ys: [f32; MAX],
    /// Tangents at the points.
    ms: [f32; MAX],
}

const fn padded(a: f32, b: f32) -> [f32; MAX] {
    let mut v = [0.0; MAX];
    v[0] = a;
    v[1] = b;
    v
}

impl Default for ResponseCurve {
    fn default() -> Self {
        Self::LINEAR
    }
}

impl ResponseCurve {
    pub const MAX_POINTS: usize = MAX;

    /// The identity: input passes through unchanged.
    pub const LINEAR: Self = Self { n: 2, xs: padded(0.0, 1.0), ys: padded(0.0, 1.0), ms: padded(1.0, 1.0) };

    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut pts: Vec<[f32; 2]> =
            points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
        pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
        pts.dedup_by(|b, a| {
            // Same input: keep the later point's output.
            let same = (a[0] - b[0]).abs() < 1e-6;
            if same {
                a[1] = b[1];
            }
            same
        });
        pts.truncate(MAX);
        let n = pts.len();
        if n < 2 {
            return Self::LINEAR;
        }
        let mut c = Self { n: n as u8, xs: [0.0; MAX], ys: [0.0; MAX], ms: [0.0; MAX] };
        let mut prev = 0.0f32;
        for (k, p) in pts.iter().enumerate() {
            c.xs[k] = p[0];
            // Outputs never fall: the curve is monotone.
            prev = if k == 0 { p[1] } else { p[1].max(prev) };
            c.ys[k] = prev;
        }
        // Fritsch–Carlson tangents: secant averages, zero at flat steps, limited so the cubic
        // never overshoots (which keeps it monotone).
        let mut d = [0.0f32; MAX];
        for (k, dk) in d.iter_mut().enumerate().take(n - 1) {
            *dk = (c.ys[k + 1] - c.ys[k]) / (c.xs[k + 1] - c.xs[k]);
        }
        c.ms[0] = d[0];
        c.ms[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            c.ms[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) / 2.0 };
        }
        for (k, &dk) in d.iter().enumerate().take(n - 1) {
            if dk == 0.0 {
                c.ms[k] = 0.0;
                c.ms[k + 1] = 0.0;
                continue;
            }
            let (a, b) = (c.ms[k] / dk, c.ms[k + 1] / dk);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                c.ms[k] = t * a * dk;
                c.ms[k + 1] = t * b * dk;
            }
        }
        c
    }

    /// The control points (sanitised).
    pub fn points(&self) -> Vec<[f32; 2]> {
        self.xs.iter().zip(&self.ys).take(usize::from(self.n)).map(|(&x, &y)| [x, y]).collect()
    }

    /// The curve's output for input `x` (0..1; anything else is clamped, NaN reads as 0).
    pub fn eval(&self, x: f32) -> f32 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        if self.is_linear() {
            return x;
        }
        let n = usize::from(self.n).min(MAX);
        let (xs, ys, ms) = (self.xs.get(..n).unwrap_or(&[]), self.ys.get(..n).unwrap_or(&[]), self.ms.get(..n).unwrap_or(&[]));
        let (Some(&x0), Some(&xn)) = (xs.first(), xs.last()) else { return x };
        if x <= x0 {
            return ys.first().copied().unwrap_or(x);
        }
        if x >= xn {
            return ys.last().copied().unwrap_or(x);
        }
        let k = xs.partition_point(|&v| v <= x).saturating_sub(1).min(n.saturating_sub(2));
        let (Some(&xa), Some(&xb), Some(&ya), Some(&yb), Some(&ma), Some(&mb)) = (xs.get(k), xs.get(k + 1), ys.get(k), ys.get(k + 1), ms.get(k), ms.get(k + 1))
        else {
            return x;
        };
        let h = xb - xa;
        let t = (x - xa) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * ya + (t3 - 2.0 * t2 + t) * h * ma + (-2.0 * t3 + 3.0 * t2) * yb + (t3 - t2) * h * mb;
        y.clamp(ya.min(yb), ya.max(yb))
    }

    /// The identity (the default curve).
    pub fn is_linear(&self) -> bool {
        self.n == 2 && self.xs[..2] == [0.0, 1.0] && self.ys[..2] == [0.0, 1.0]
    }
}

impl Serialize for ResponseCurve {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.points().serialize(s)
    }
}

impl<'de> Deserialize<'de> for ResponseCurve {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(&Vec::<[f32; 2]>::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_is_monotone_sanitised_and_round_trips_as_points() {
        assert!(ResponseCurve::new(&[[0.0, 0.0], [1.0, 1.0]]).is_linear());
        assert_eq!(ResponseCurve::new(&[[0.0, 0.0], [1.0, 1.0]]), ResponseCurve::LINEAR);
        assert!(ResponseCurve::new(&[[f32::NAN, 0.3]]).is_linear(), "fewer than two usable points is linear");
        let c = ResponseCurve::new(&[[1.0, 1.0], [0.5, 0.2], [0.0, 0.0]]);
        assert!((c.eval(0.5) - 0.2).abs() < 1e-6);
        let mut prev = 0.0;
        for i in 0..=100 {
            let y = c.eval(i as f32 / 100.0);
            assert!(y >= prev - 1e-6 && (0.0..=1.0).contains(&y));
            prev = y;
        }
        let json = serde_json::to_string(&c).unwrap_or_default();
        assert_eq!(json, "[[0.0,0.0],[0.5,0.2],[1.0,1.0]]");
        let back: ResponseCurve = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(back, c);
        let many: Vec<[f32; 2]> = (0..40).map(|i| [i as f32 / 40.0, 0.5]).collect();
        assert_eq!(ResponseCurve::new(&many).points().len(), ResponseCurve::MAX_POINTS);
    }
}
