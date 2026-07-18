//! Pure, deterministic per-decision credit-quality score `q in [0,1]`, computed
//! from the gate's stored numeric signals. Multiplicative and log-concave
//! (anti-Goodhart), with a peak-vs-representative anomaly term used ONLY as a
//! fraud flag — never a bonus. Shadow-only: nothing here settles or pays.

/// Pinned, versioned calibration constants. `*_CEIL` and `anomaly_*` are
/// calibration outputs; the V1 defaults are seeded from the 2026-07-12 27B
/// distribution (perplexity p90 ~= 38.5) and refined by the on-pilot
/// distribution run (see the design spec's rollout). Bumping any value MUST
/// bump `version`.
#[derive(Debug, Clone, Copy)]
pub struct CreditQualityConstants {
    pub ppl_floor_micros: i64,
    pub ppl_ceil_micros: i64,
    pub nov_floor_micros: i64,
    pub nov_ceil_micros: i64,
    /// Graded-floor multiplier for the perplexity term, * 1e6. A below-floor
    /// perplexity yields this fraction of the term instead of 0. `0` reproduces
    /// the pre-V2 hard-zero behavior exactly.
    pub ppl_floor_mult_micros: i64,
    /// Graded-floor multiplier for the novelty term, * 1e6. See above.
    pub nov_floor_mult_micros: i64,
    pub anomaly_soft_ratio_micros: i64,
    pub anomaly_hard_ratio_micros: i64,
    pub version: i32,
}

pub const CREDIT_QUALITY_CONSTANTS_V1: CreditQualityConstants = CreditQualityConstants {
    ppl_floor_micros: 6_000_000,
    ppl_ceil_micros: 38_500_000,
    nov_floor_micros: 500_000,
    nov_ceil_micros: 1_000_000,
    // V1 hard-zeroes below floor: floor_mult == 0 makes the affine wrapper the
    // identity, so V1 scores are byte-identical to the pre-V2 formula.
    ppl_floor_mult_micros: 0,
    nov_floor_mult_micros: 0,
    anomaly_soft_ratio_micros: 3_000_000, // r <= 3.0 -> no penalty
    anomaly_hard_ratio_micros: 10_000_000, // r >= 10.0 -> withhold
    version: 1,
};

/// V2 softens the perplexity and novelty terms from hard-zeroing floors to
/// graded (affine) floors: substantive-but-unremarkable work earns a nonzero
/// quality signal instead of 0. Safe now that cross-trace dedup (`dup_pen`,
/// PR #169) carries the anti-duplication load the novelty term used to bear.
/// Floors/ceilings/anomaly thresholds are unchanged from V1; the floor-mults are
/// calibration starting points to tune on the pilot backfill. The anomaly term
/// remains a hard fraud gate.
pub const CREDIT_QUALITY_CONSTANTS_V2: CreditQualityConstants = CreditQualityConstants {
    ppl_floor_mult_micros: 250_000, // 0.25
    nov_floor_mult_micros: 300_000, // 0.30
    version: 2,
    ..CREDIT_QUALITY_CONSTANTS_V1
};

/// The active calibration used by the production inline score and the batch
/// re-score route. Bumping the active version touches this one line.
pub const CREDIT_QUALITY_ACTIVE: CreditQualityConstants = CREDIT_QUALITY_CONSTANTS_V2;

/// Result of scoring one decision row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditQualityScore {
    /// `q` * 1e6, clamped to [0, 1_000_000].
    pub q_micros: i64,
    /// peak/rep spikiness ratio * 1e6 (>= 0; defaults to 1_000_000 when rep <= 0).
    pub anomaly_ratio_micros: i64,
    /// True when the spikiness ratio reached the hard threshold (a == 0).
    pub anomaly_withheld: bool,
}

/// Concave, saturating map of a micros signal onto [0,1]:
/// `log(1 + max(0, x - floor)) / log(1 + ceil - floor)`, in real (non-micros)
/// units. Below floor -> 0; at/above ceil -> 1.
fn saturating_term(value_micros: i64, floor_micros: i64, ceil_micros: i64) -> f64 {
    if value_micros <= floor_micros {
        return 0.0;
    }
    let x = (value_micros - floor_micros) as f64 / 1_000_000.0;
    // max(1) guards a degenerate ceil <= floor against divide-by-zero.
    let span = ((ceil_micros - floor_micros).max(1)) as f64 / 1_000_000.0;
    ((1.0 + x).ln() / (1.0 + span).ln()).clamp(0.0, 1.0)
}

/// Apply a graded (affine) floor to a `[0,1]` saturating-term value:
/// `floor_mult + (1 - floor_mult) * sat`. `floor_mult == 0` is the identity
/// (hard-zero, V1 behavior); a below-floor signal yields `floor_mult` instead of
/// 0 while a ceiling-saturated signal still yields 1.0. Monotonicity and
/// concavity of `sat` are preserved.
fn graded_floor(sat: f64, floor_mult_micros: i64) -> f64 {
    let floor_mult = (floor_mult_micros as f64 / 1_000_000.0).clamp(0.0, 1.0);
    floor_mult + (1.0 - floor_mult) * sat
}

pub fn credit_quality(
    ppl_rep_micros: i64,
    ppl_peak_micros: i64,
    nov_rep_micros: i64,
    k: &CreditQualityConstants,
) -> CreditQualityScore {
    let f = graded_floor(
        saturating_term(ppl_rep_micros, k.ppl_floor_micros, k.ppl_ceil_micros),
        k.ppl_floor_mult_micros,
    );
    let g = graded_floor(
        saturating_term(nov_rep_micros, k.nov_floor_micros, k.nov_ceil_micros),
        k.nov_floor_mult_micros,
    );

    // Spikiness ratio r = peak / rep (real units); rep <= 0 -> ratio 1.0 (no signal).
    let (ratio, anomaly_ratio_micros) = if ppl_rep_micros <= 0 {
        (1.0_f64, 1_000_000_i64)
    } else {
        let r = ppl_peak_micros.max(0) as f64 / ppl_rep_micros as f64;
        (r, (r * 1_000_000.0).round() as i64)
    };

    let soft = k.anomaly_soft_ratio_micros as f64 / 1_000_000.0;
    let hard = k.anomaly_hard_ratio_micros as f64 / 1_000_000.0;
    let (a, withheld) = if ratio <= soft {
        (1.0, false)
    } else if ratio >= hard {
        (0.0, true)
    } else {
        (1.0 - (ratio - soft) / (hard - soft), false)
    };

    let q = (f * g * a).clamp(0.0, 1.0);
    CreditQualityScore {
        q_micros: (q * 1_000_000.0).round() as i64,
        anomaly_ratio_micros,
        anomaly_withheld: withheld,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: real perplexity/novelty -> micros
    fn m(x: f64) -> i64 {
        (x * 1_000_000.0).round() as i64
    }
    const K: CreditQualityConstants = CREDIT_QUALITY_CONSTANTS_V1;

    #[test]
    fn below_floor_scores_zero() {
        // perplexity below floor (6.0) -> f = 0 -> q = 0, regardless of novelty
        let s = credit_quality(m(5.9), m(6.0), m(0.9), &K);
        assert_eq!(s.q_micros, 0);
        // novelty below floor (0.5) -> g = 0 -> q = 0, regardless of perplexity
        let s = credit_quality(m(20.0), m(21.0), m(0.49), &K);
        assert_eq!(s.q_micros, 0);
    }

    #[test]
    fn at_or_above_ceiling_saturates_to_one_per_term() {
        // both signals at/above ceiling, low spikiness -> q == 1_000_000
        let s = credit_quality(K.ppl_ceil_micros, K.ppl_ceil_micros, K.nov_ceil_micros, &K);
        assert_eq!(s.q_micros, 1_000_000);
        // the 1642 outlier saturates the same as p90 (log ceiling is the winsorizer)
        let outlier = credit_quality(m(1642.0), m(1642.0), K.nov_ceil_micros, &K);
        let at_ceil = credit_quality(K.ppl_ceil_micros, K.ppl_ceil_micros, K.nov_ceil_micros, &K);
        assert_eq!(outlier.q_micros, at_ceil.q_micros);
    }

    #[test]
    fn multiplicative_collapse_one_signal_low() {
        // high perplexity but novelty just above floor -> g near 0 -> q small
        let s = credit_quality(m(30.0), m(31.0), m(0.51), &K);
        assert!(
            s.q_micros < 100_000,
            "expected collapse, got {}",
            s.q_micros
        );
    }

    #[test]
    fn monotonic_nondecreasing_in_perplexity() {
        let a = credit_quality(m(8.0), m(8.0), m(0.9), &K).q_micros;
        let b = credit_quality(m(12.0), m(12.0), m(0.9), &K).q_micros;
        assert!(
            b >= a,
            "q must not decrease as perplexity rises: {a} then {b}"
        );
    }

    #[test]
    fn concave_diminishing_returns() {
        // equal input steps yield non-increasing output steps (concavity)
        let q = |p: f64| credit_quality(m(p), m(p), m(0.9), &K).q_micros;
        let d1 = q(10.0) - q(8.0);
        let d2 = q(12.0) - q(10.0);
        assert!(d2 <= d1, "expected concavity: d1={d1} d2={d2}");
    }

    #[test]
    fn anomaly_soft_no_penalty_hard_withholds() {
        // r <= soft -> a = 1 (no penalty): peak == rep
        let no_pen = credit_quality(m(20.0), m(20.0), m(0.9), &K);
        // r >= hard -> a = 0 -> q = 0 + withheld flag: huge peak vs tiny rep
        let hard_r = (K.anomaly_hard_ratio_micros as f64 / 1_000_000.0) + 1.0;
        let withheld = credit_quality(m(7.0), m(7.0 * hard_r), m(0.9), &K);
        assert!(no_pen.q_micros > 0);
        assert_eq!(withheld.q_micros, 0);
        assert!(withheld.anomaly_withheld);
        assert!(!no_pen.anomaly_withheld);
    }

    #[test]
    fn anomaly_ratio_is_reported() {
        let s = credit_quality(m(10.0), m(25.0), m(0.9), &K);
        // ratio = peak/rep = 2.5 -> 2_500_000 micros (allow rounding slack)
        assert!((s.anomaly_ratio_micros - 2_500_000).abs() <= 2);
    }

    #[test]
    fn zero_or_negative_rep_is_safe() {
        // rep == 0 -> below floor -> q = 0, ratio defaults to 1.0 (no divide-by-zero)
        let s = credit_quality(0, 0, m(0.9), &K);
        assert_eq!(s.q_micros, 0);
        assert_eq!(s.anomaly_ratio_micros, 1_000_000);
    }

    #[test]
    fn deterministic() {
        let a = credit_quality(m(11.0), m(14.0), m(0.8), &K);
        let b = credit_quality(m(11.0), m(14.0), m(0.8), &K);
        assert_eq!(a.q_micros, b.q_micros);
        assert_eq!(a.anomaly_ratio_micros, b.anomaly_ratio_micros);
    }

    #[test]
    fn genuine_beats_every_gamed_variant() {
        // genuine: both mid-high, low spikiness
        let genuine = credit_quality(m(15.0), m(18.0), m(0.85), &K).q_micros;
        // rare-token pump: very high ppl, novelty just above floor
        let pump = credit_quality(m(1642.0), m(1642.0), m(0.51), &K).q_micros;
        // distinctive-token shim: high novelty, ppl just above floor
        let shim = credit_quality(m(6.2), m(6.4), m(0.99), &K).q_micros;
        // peak parasite: low rep, huge peak
        let parasite = credit_quality(m(6.5), m(120.0), m(0.85), &K).q_micros;
        assert!(genuine > pump, "genuine {genuine} !> pump {pump}");
        assert!(genuine > shim, "genuine {genuine} !> shim {shim}");
        assert!(
            genuine > parasite,
            "genuine {genuine} !> parasite {parasite}"
        );
    }

    // ---- V2 (graded affine floor) ----
    const K2: CreditQualityConstants = CREDIT_QUALITY_CONSTANTS_V2;

    #[test]
    fn v1_floor_mults_are_zero_so_v2_formula_reproduces_v1() {
        // With floor_mult == 0 the affine wrapper is the identity, so V1
        // scores must be byte-identical to the pre-V2 hard-zero behavior.
        assert_eq!(K.ppl_floor_mult_micros, 0);
        assert_eq!(K.nov_floor_mult_micros, 0);
        // below-floor still zeroes under V1
        assert_eq!(credit_quality(m(5.9), m(6.0), m(0.9), &K).q_micros, 0);
        assert_eq!(credit_quality(m(20.0), m(21.0), m(0.49), &K).q_micros, 0);
    }

    #[test]
    fn v2_graded_floor_lifts_below_floor_off_zero() {
        // A non-anomalous trace below BOTH floors scores the product of the two
        // floor multipliers, not zero: 0.25 * 0.30 = 0.075 -> 75_000 micros.
        let s = credit_quality(m(5.0), m(5.0), m(0.4), &K2);
        assert_eq!(
            s.q_micros, 75_000,
            "expected floor product, got {}",
            s.q_micros
        );
        assert!(!s.anomaly_withheld);
    }

    #[test]
    fn v2_saturates_to_one_at_ceilings() {
        // both signals at/above ceiling, low spikiness -> still exactly 1.0
        let s = credit_quality(
            K2.ppl_ceil_micros,
            K2.ppl_ceil_micros,
            K2.nov_ceil_micros,
            &K2,
        );
        assert_eq!(s.q_micros, 1_000_000);
    }

    #[test]
    fn v2_anomaly_still_hard_zeroes() {
        // Even with softened floors, the hard spikiness ratio zeroes q and flags.
        let hard_r = (K2.anomaly_hard_ratio_micros as f64 / 1_000_000.0) + 1.0;
        let s = credit_quality(m(7.0), m(7.0 * hard_r), m(0.9), &K2);
        assert_eq!(s.q_micros, 0);
        assert!(s.anomaly_withheld);
    }

    #[test]
    fn v2_monotonic_and_concave_in_perplexity() {
        let q = |p: f64| credit_quality(m(p), m(p), m(0.9), &K2).q_micros;
        assert!(q(12.0) >= q(8.0), "monotonic");
        let d1 = q(10.0) - q(8.0);
        let d2 = q(12.0) - q(10.0);
        assert!(d2 <= d1, "expected concavity: d1={d1} d2={d2}");
    }

    #[test]
    fn v2_genuine_beats_every_gamed_variant() {
        // The anti-gaming sanity check must still hold under the softened floors.
        let genuine = credit_quality(m(15.0), m(18.0), m(0.85), &K2).q_micros;
        let pump = credit_quality(m(1642.0), m(1642.0), m(0.51), &K2).q_micros;
        let shim = credit_quality(m(6.2), m(6.4), m(0.99), &K2).q_micros;
        let parasite = credit_quality(m(6.5), m(120.0), m(0.85), &K2).q_micros;
        assert!(genuine > pump, "genuine {genuine} !> pump {pump}");
        assert!(genuine > shim, "genuine {genuine} !> shim {shim}");
        assert!(
            genuine > parasite,
            "genuine {genuine} !> parasite {parasite}"
        );
    }

    #[test]
    fn v2_property_monotonic_and_bounded() {
        let novs = [m(0.2), m(0.6), m(1.0)];
        for &nov in &novs {
            let mut prev = -1i64;
            let mut p = 0.0_f64;
            while p <= 60.0 {
                let q = credit_quality(m(p), m(p), nov, &K2).q_micros;
                assert!((0..=1_000_000).contains(&q), "q out of range: {q}");
                assert!(
                    q >= prev,
                    "not monotonic at p={p} nov={nov}: {prev} then {q}"
                );
                prev = q;
                p += 0.5;
            }
        }
    }

    // Property-based layer (loop-sampled, no new dependency).
    #[test]
    fn property_monotonic_and_bounded() {
        let novs = [m(0.6), m(0.8), m(1.0)];
        for &nov in &novs {
            let mut prev = -1i64;
            let mut p = 6.0_f64;
            while p <= 60.0 {
                let q = credit_quality(m(p), m(p), nov, &K).q_micros;
                assert!((0..=1_000_000).contains(&q), "q out of range: {q}");
                assert!(
                    q >= prev,
                    "not monotonic at p={p} nov={nov}: {prev} then {q}"
                );
                prev = q;
                p += 0.5;
            }
        }
    }
}
