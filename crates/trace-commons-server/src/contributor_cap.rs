//! Pure, deterministic per-decision contributor-cap factor. A saturating
//! concave function of a contributor's in-epoch cumulative raw credit
//! `R = sum(q * dup_pen)` bounds how much any one identity can earn per epoch:
//! `effective(R) = K * (1 - exp(-R/K))`, asymptoting to `K`. Each decision's
//! `contributor_factor` is the MARGINAL effective-per-raw at its point in the
//! running total, decaying from 1.0 (first trace of the epoch) toward 0.
//! Shadow-only: nothing here settles or pays. Epochs are fixed 7-day buckets.

#[derive(Debug, Clone, Copy)]
pub struct ContributorCapConstants {
    /// Per-epoch asymptote K * 1e6 (max total effective credit one identity
    /// can earn in a single epoch, regardless of how many traces it submits).
    pub k_micros: i64,
    /// Epoch length in days; `R` resets at each epoch boundary.
    pub epoch_days: i64,
    pub version: i32,
}

pub const CONTRIBUTOR_CAP_CONSTANTS_V1: ContributorCapConstants = ContributorCapConstants {
    k_micros: 25_000_000, // K = 25.0 per epoch; calibration seed, tune on backfill
    epoch_days: 7,
    version: 1,
};

/// Deterministic epoch bucket of a decision's `decided_at` (unix seconds).
/// `floor(secs / (epoch_days * 86400))` — globally consistent, no genesis anchor.
pub fn epoch_index(decided_at_unix_secs: i64, epoch_days: i64) -> i64 {
    let epoch_secs = epoch_days.max(1) * 86_400;
    decided_at_unix_secs.div_euclid(epoch_secs)
}

/// This decision's raw pipeline increment `r = q * dup_pen`, in micros.
/// `dup_pen = 1 / dedup_cluster_size` (NULL size -> 1); NULL q -> 0. Because
/// `q_micros` is already micros and `dup_pen` is a fraction, `q_micros / size`
/// stays on the micros scale.
pub fn increment_micros(
    credit_quality_micros: Option<i64>,
    dedup_cluster_size: Option<i32>,
) -> i64 {
    let q = credit_quality_micros.unwrap_or(0).max(0);
    let size = dedup_cluster_size.unwrap_or(1).max(1) as i64;
    q / size
}

/// Marginal cap factor (* 1e6, clamped to [0,1e6]) for a decision with
/// increment `r_micros` at prior in-epoch cumulative `r_before_micros`:
/// `(effective(R_before + r) - effective(R_before)) / r` for `r > 0`, else the
/// `r -> 0` derivative limit `exp(-R_before / K)`.
pub fn contributor_factor_micros(
    r_before_micros: i64,
    r_micros: i64,
    k: &ContributorCapConstants,
) -> i64 {
    let kf = k.k_micros.max(1) as f64 / 1_000_000.0;
    let rb = r_before_micros.max(0) as f64 / 1_000_000.0;
    let r = r_micros.max(0) as f64 / 1_000_000.0;
    let factor = if r <= 0.0 {
        (-rb / kf).exp()
    } else {
        let eff = |x: f64| kf * (1.0 - (-x / kf).exp());
        (eff(rb + r) - eff(rb)) / r
    };
    (factor.clamp(0.0, 1.0) * 1_000_000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    const K: ContributorCapConstants = CONTRIBUTOR_CAP_CONSTANTS_V1;
    fn m(x: f64) -> i64 {
        (x * 1_000_000.0).round() as i64
    }

    #[test]
    fn epoch_buckets_are_seven_day_windows() {
        let day = 86_400;
        // Two timestamps 6 days apart share an epoch; 8 days apart do not,
        // when aligned to bucket boundaries.
        assert_eq!(epoch_index(0, 7), 0);
        assert_eq!(epoch_index(6 * day, 7), 0);
        assert_eq!(epoch_index(7 * day, 7), 1);
        assert_eq!(epoch_index(15 * day, 7), 2);
    }

    #[test]
    fn increment_applies_dup_pen() {
        // r = q * (1/size). q=0.8, size=4 -> 0.2
        assert_eq!(increment_micros(Some(m(0.8)), Some(4)), m(0.2));
        // NULL size -> dup_pen 1; NULL q -> 0
        assert_eq!(increment_micros(Some(m(0.8)), None), m(0.8));
        assert_eq!(increment_micros(None, Some(4)), 0);
    }

    #[test]
    fn first_trace_of_epoch_has_full_factor() {
        // R_before = 0 -> marginal factor ~ 1.0 for a small increment.
        let f = contributor_factor_micros(0, m(0.1), &K);
        assert!(f > 990_000, "expected near-1.0, got {f}");
    }

    #[test]
    fn factor_decays_as_cumulative_rises() {
        let early = contributor_factor_micros(0, m(0.1), &K);
        let mid = contributor_factor_micros(m(20.0), m(0.1), &K);
        let late = contributor_factor_micros(m(60.0), m(0.1), &K);
        assert!(
            early > mid && mid > late,
            "monotone decay: {early} {mid} {late}"
        );
    }

    #[test]
    fn zero_increment_uses_derivative_limit() {
        // r == 0 -> exp(-R_before/K). At R_before = K, that's exp(-1) ~ 0.3679.
        let f = contributor_factor_micros(K.k_micros, 0, &K);
        assert!((f - 367_879).abs() <= 200, "exp(-1) limit, got {f}");
    }

    #[test]
    fn epoch_total_effective_is_bounded_by_k() {
        // Telescoping: sum of (r * factor) over an epoch == effective(R_final) <= K.
        // Flood 400 increments of 0.2 each (R_final = 80, well past K=25).
        let mut r_before = 0i64;
        let mut total_effective = 0.0f64;
        for _ in 0..400 {
            let r = m(0.2);
            let factor = contributor_factor_micros(r_before, r, &K) as f64 / 1_000_000.0;
            total_effective += (r as f64 / 1_000_000.0) * factor;
            r_before += r;
        }
        let k = K.k_micros as f64 / 1_000_000.0;
        assert!(
            total_effective <= k + 1e-6,
            "total {total_effective} must be <= K {k}"
        );
        // And it should be close to K (saturated), not far below.
        // With R_final=80 and K=25, we achieve ~95.9% saturation (not 99%)
        assert!(
            total_effective > k * 0.95,
            "expected saturation near K, got {total_effective}"
        );
    }

    #[test]
    fn deterministic_and_bounded() {
        let a = contributor_factor_micros(m(13.0), m(0.15), &K);
        let b = contributor_factor_micros(m(13.0), m(0.15), &K);
        assert_eq!(a, b);
        assert!((0..=1_000_000).contains(&a));
    }
}
