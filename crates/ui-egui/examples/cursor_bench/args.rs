/// Validate before allocating samples. Twenty-four cases retain three timing vectors,
/// so 20,000 samples per case bounds raw timing storage to about 12 MiB.
pub(super) fn checked_sample_count(moves: usize, reps: usize) -> Result<usize, &'static str> {
    if moves == 0 || moves > 100_000 {
        return Err("moves count must be in 1..=100000");
    }
    if reps == 0 || reps > 100 {
        return Err("reps count must be in 1..=100");
    }
    let count = moves.checked_mul(reps).ok_or("move count overflow")?;
    if count > 20_000 {
        return Err("moves * reps must not exceed 20000 samples per cursor case");
    }
    Ok(count)
}

#[cfg(test)]
mod argument_tests {
    use super::checked_sample_count;

    #[test]
    fn sample_budget_accepts_defaults_and_boundary_but_rejects_excess() {
        assert_eq!(checked_sample_count(1000, 5), Ok(5000));
        assert_eq!(checked_sample_count(200, 2), Ok(400));
        assert_eq!(checked_sample_count(200, 100), Ok(20_000));
        assert!(checked_sample_count(10_001, 2).is_err());
        assert!(checked_sample_count(100_000, 100).is_err());
        assert!(checked_sample_count(usize::MAX, 2).is_err());
        assert!(checked_sample_count(0, 1).is_err());
        assert!(checked_sample_count(1, 0).is_err());
    }
}
