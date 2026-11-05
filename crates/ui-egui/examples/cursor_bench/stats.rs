/// Read one nearest-rank percentile from ascending-sorted timing samples: rank `ceil(pct * n)`,
/// clamped to `1..=n`. Twelve samples therefore report the largest value at p95, not the eleventh.
/// An empty slice reports `0.0`; a NaN percentage reports the lowest sample and an infinite one
/// the highest, so hostile input stays inside the bounds.
pub(super) fn percentile(sorted: &[f64], pct: f64) -> f64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0.0;
    };
    let rank = (pct * (last + 1) as f64).ceil();
    let rank = if rank.is_nan() { 1.0 } else { rank.clamp(1.0, (last + 1) as f64) };
    sorted[rank as usize - 1]
}
