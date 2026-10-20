//! Reducing captured per-token log-probabilities to a few numbers.
//!
//! # Why reduce at all
//!
//! Raw per-token distributions cannot travel. Trace Commons caps an ingest body
//! at 2 MiB and a typical trace's log-probabilities are several times that, so
//! anything that wants to carry this signal has to reduce it first. IronWire is
//! where the raw data already is, which makes it the only place the reduction
//! can happen without the distributions leaving the machine.
//!
//! The privacy argument runs the same way. Distributions are conditioned on the
//! entire context, which makes them more sensitive than the text they describe;
//! four aggregate numbers give an attacker essentially nothing to invert.
//!
//! # Log-probabilities in, probabilities out
//!
//! [`reduce_token_logprobs`] takes **log-probabilities** and exponentiates once,
//! here, in `f64`. The alternative — the shape this started as — is to
//! exponentiate each token into an `f32` as it streams in and take the
//! arithmetic mean of those. That is wrong twice over.
//!
//! Summing is the measurable one. Two hundred thousand tokens of `exp(-10)`
//! summed in `f32` gives a mean 0.04% off the true value, because the running
//! total grows large relative to each addend; the same sum in `f64` is exact to
//! sixteen digits. Agent turns really are that long, and 0.04% is far more than
//! an `f32` result can excuse.
//!
//! Representation is the sharper one. `exp` of a log-probability below about
//! `-87` leaves `f32`'s normal range and starts shedding mantissa bits; below
//! about `-104` it is exactly zero. Those are the genuinely surprising tokens —
//! precisely what this feature exists to localise — and narrowing per token
//! destroys them before anything gets to look.
//!
//! # These are not dataset cartography
//!
//! The vocabulary comes from Swayamdipta et al., *Dataset Cartography* (2020),
//! which defines confidence, variability and correctness **across training
//! epochs against a gold label**. Single-pass generation has neither epochs nor
//! a gold label.
//!
//! | Field | Cartography | Here |
//! |---|---|---|
//! | `mean_confidence` | mean p(gold) across epochs | mean p(chosen) across the trace |
//! | `variability` | s.d. of p(gold) across **epochs** | s.d. of p(chosen) across **tokens** |
//! | `correctness` | fraction of epochs predicted right | not derivable here at all |
//!
//! `variability` differs in kind rather than degree, and the two must not be
//! compared. The names match the Trace Commons envelope field they feed, which
//! is the only reason to keep them.

use serde::{Deserialize, Serialize};

/// At or above this mean confidence, a steady trace is [`ConfidenceBucket::Easy`].
///
/// Provisional rather than calibrated: no corpus has been measured against it.
/// **Mirrors the constant of the same name in `trace_commons_protocol`** — the
/// two must move together, or producer and server disagree about what a bucket
/// means. Nothing in this workspace can check that, because nothing here
/// depends on that crate; `the_bucket_thresholds_are_pinned` is what makes a
/// change to either side visible instead of silent.
pub const EASY_MEAN_CONFIDENCE: f32 = 0.75;

/// At or above this dispersion a trace is [`ConfidenceBucket::Ambiguous`]
/// whatever its mean. Mirrors the server constant; see above.
pub const AMBIGUOUS_VARIABILITY: f32 = 0.25;

/// Coarse shape of a trace's confidence profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBucket {
    /// Confident throughout: the model was rarely in doubt.
    Easy,
    /// Swung between certainty and doubt. Checked before the mean, because
    /// averaging is exactly what hides this case.
    Ambiguous,
    /// Steadily unconfident: uniformly unsure rather than torn.
    Hard,
}

impl ConfidenceBucket {
    /// The name this bucket is stored and reported under.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Ambiguous => "ambiguous",
            Self::Hard => "hard",
        }
    }

    /// Read a bucket back from the name [`Self::as_str`] wrote.
    ///
    /// `None` for anything else — including a name a *newer* IronWire wrote
    /// into the same ledger file. An unrecognised bucket is an absent one, not
    /// a guessed one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "easy" => Some(Self::Easy),
            "ambiguous" => Some(Self::Ambiguous),
            "hard" => Some(Self::Hard),
            _ => None,
        }
    }
}

/// Aggregate confidence signals for one exchange.
///
/// `correctness` is deliberately absent: no arrangement of log-probabilities
/// answers whether the work was right. It needs an outcome signal, and whatever
/// assembles a contribution supplies it separately rather than letting
/// confidence stand in for it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceAggregates {
    /// Mean probability of the tokens the model emitted, in `0.0..=1.0`.
    pub mean_confidence: f32,
    /// Population standard deviation of those probabilities across tokens.
    pub variability: f32,
    /// Bucket derived from the two figures above.
    pub bucket: ConfidenceBucket,
    /// How many tokens the aggregate is over. A mean over eleven tokens and a
    /// mean over eleven thousand are not the same claim, and a consumer that
    /// cannot tell them apart will treat them as though they were.
    pub token_count: usize,
}

/// Reduce per-token log-probabilities to aggregates.
///
/// `logprobs` are `ln p(chosen token)`, straight off the wire. Returns `None`
/// for an empty slice: nothing observed means nothing claimed, and a
/// confident-looking zero would be worse than an absence.
///
/// A value that is not a log-probability — not finite, or above zero, which
/// would exponentiate past 1 — is ignored rather than clamped. A backend
/// emitting one is wrong in a way we should not paper over, and folding a
/// repaired value into the mean would bias it toward whichever bound the repair
/// chose and hide the bug for good.
///
/// Everything after the filter is `f64` until the last step; see the module
/// note on why the order of operations is the whole point.
#[must_use]
pub fn reduce_token_logprobs(logprobs: &[f64]) -> Option<ConfidenceAggregates> {
    let probabilities: Vec<f64> = logprobs
        .iter()
        .filter(|logprob| logprob.is_finite() && **logprob <= 0.0)
        .map(|logprob| logprob.exp())
        .collect();
    if probabilities.is_empty() {
        return None;
    }

    let count = probabilities.len() as f64;
    let mean = (probabilities.iter().sum::<f64>() / count).clamp(0.0, 1.0);
    let variance = probabilities
        .iter()
        .map(|p| (p - mean).powi(2))
        .sum::<f64>()
        / count;
    let variability = variance.sqrt().clamp(0.0, 1.0);

    // Narrowed once, at the end, after every sum is done.
    let mean = mean as f32;
    let variability = variability as f32;

    let bucket = if variability >= AMBIGUOUS_VARIABILITY {
        // Dispersion first: a run that swings between certainty and doubt is
        // the interesting case, and its mean hides exactly that.
        ConfidenceBucket::Ambiguous
    } else if mean >= EASY_MEAN_CONFIDENCE {
        ConfidenceBucket::Easy
    } else {
        ConfidenceBucket::Hard
    };

    Some(ConfidenceAggregates {
        mean_confidence: mean,
        variability,
        bucket,
        token_count: probabilities.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected ~{expected}, got {actual}"
        );
    }

    /// Nothing observed means nothing claimed. Capture is off by default, so
    /// this is the overwhelmingly common case and it must not look like a
    /// measured zero.
    #[test]
    fn nothing_captured_yields_nothing() {
        assert!(reduce_token_logprobs(&[]).is_none());
    }

    #[test]
    fn all_values_unusable_yields_nothing() {
        assert!(reduce_token_logprobs(&[f64::NAN, 2.0, f64::INFINITY]).is_none());
    }

    /// The hand-computed case, carried over from the version that accumulated
    /// probabilities directly: p = 0.2, 0.4, 0.6, 0.8 has mean 0.5 and
    /// population s.d. 0.223607. Accumulating in `f64` and exponentiating once
    /// must not move it.
    #[test]
    fn mean_and_variability_are_the_population_statistics() {
        let logprobs: Vec<f64> = [0.2, 0.4, 0.6, 0.8].iter().map(|p: &f64| p.ln()).collect();
        let a = reduce_token_logprobs(&logprobs).expect("aggregates");
        approx(a.mean_confidence, 0.5);
        approx(a.variability, 0.223_607);
        assert_eq!(a.token_count, 4);
    }

    /// What accumulating in `f32` costs, measured rather than asserted. Two
    /// hundred thousand tokens of `p = exp(-10)`: an `f32` running sum drifts
    /// once it is large relative to each addend, and lands 0.04% off. The
    /// naive sum is computed here as well, so this fails if the implementation
    /// ever becomes the naive one — and the second assertion fails if the
    /// drift it is guarding against stops being real.
    #[test]
    fn the_mean_is_accumulated_in_f64_rather_than_f32() {
        let logprobs = vec![-10.0f64; 200_000];
        let exact = (-10.0f64).exp();

        let a = reduce_token_logprobs(&logprobs).expect("aggregates");
        assert!(
            (f64::from(a.mean_confidence) - exact).abs() < 1e-11,
            "expected {exact:e}, got {:e}",
            a.mean_confidence
        );

        let naive: f32 = logprobs.iter().map(|l| l.exp() as f32).sum::<f32>() / 200_000.0;
        assert!(
            (f64::from(naive) - exact).abs() > 1e-9,
            "the f32 drift this test guards against is no longer real: {naive:e}"
        );
    }

    /// The sharper half of the same problem: below about `-104` an `f32` cannot
    /// hold `exp(logprob)` at all. Storing log-probabilities and narrowing once
    /// at the end is what keeps those tokens alive as far as the arithmetic
    /// goes.
    #[test]
    fn a_deeply_surprising_token_is_not_narrowed_before_it_is_summed() {
        assert_eq!(
            (-110.0f64).exp() as f32,
            0.0,
            "the f32 underflow this test exists for is no longer real"
        );
        let a = reduce_token_logprobs(&[-110.0, 0.5f64.ln()]).expect("aggregates");
        assert_eq!(a.token_count, 2, "a tiny probability is a token, not a gap");
        approx(a.mean_confidence, 0.25);
    }

    #[test]
    fn a_single_token_has_no_variability() {
        let a = reduce_token_logprobs(&[0.42f64.ln()]).expect("aggregates");
        approx(a.mean_confidence, 0.42);
        approx(a.variability, 0.0);
        assert_eq!(a.token_count, 1);
    }

    /// Ignored, not clamped — otherwise a caller's bug is folded into the mean
    /// and never seen again.
    #[test]
    fn unusable_values_are_ignored_not_repaired() {
        let a =
            reduce_token_logprobs(&[0.5f64.ln(), f64::NEG_INFINITY, 0.4, f64::NAN, 0.5f64.ln()])
                .expect("aggregates");
        approx(a.mean_confidence, 0.5);
        assert_eq!(
            a.token_count, 2,
            "only the two real log-probabilities count"
        );
    }

    #[test]
    fn steady_and_confident_is_easy() {
        let a =
            reduce_token_logprobs(&[0.95f64.ln(), 0.93f64.ln(), 0.97f64.ln()]).expect("aggregates");
        assert_eq!(a.bucket, ConfidenceBucket::Easy);
    }

    #[test]
    fn steady_and_unconfident_is_hard() {
        let a =
            reduce_token_logprobs(&[0.10f64.ln(), 0.12f64.ln(), 0.08f64.ln()]).expect("aggregates");
        assert_eq!(a.bucket, ConfidenceBucket::Hard);
    }

    /// Dispersion wins regardless of the mean.
    #[test]
    fn high_variability_is_ambiguous_whatever_the_mean() {
        for probs in [
            vec![0.99, 0.01, 0.99, 0.01],
            vec![1.0, 0.4, 1.0, 0.4],
            vec![0.2, 0.9, 0.1, 0.8],
        ] {
            let logprobs: Vec<f64> = probs.iter().map(|p: &f64| p.ln()).collect();
            let a = reduce_token_logprobs(&logprobs).expect("aggregates");
            assert_eq!(a.bucket, ConfidenceBucket::Ambiguous, "for {probs:?}");
        }
    }

    /// Everything emitted here has to satisfy the bounds Trace Commons enforces
    /// on the envelope, or a contribution built from it is rejected at ingest.
    #[test]
    fn output_always_fits_the_envelope_bounds() {
        let corpora: Vec<Vec<f64>> = vec![
            vec![f64::NEG_INFINITY.max(-700.0), -700.0],
            vec![0.0, 0.0],
            vec![-700.0, 0.0],
            vec![0.5f64.ln()],
            (0..500).map(|i| -f64::from(i % 101) / 10.0).collect(),
        ];
        for logprobs in corpora {
            let a = reduce_token_logprobs(&logprobs).expect("aggregates");
            for value in [a.mean_confidence, a.variability] {
                assert!(
                    value.is_finite() && (0.0..=1.0).contains(&value),
                    "out of bounds: {value} from {logprobs:?}"
                );
            }
        }
    }

    /// The token count is what lets a consumer tell a mean over eleven tokens
    /// from a mean over eleven thousand.
    #[test]
    fn token_count_reports_what_the_mean_is_over() {
        let a = reduce_token_logprobs(&[0.5f64.ln(); 1000]).expect("aggregates");
        assert_eq!(a.token_count, 1000);
    }

    /// These two numbers mirror constants in `trace_commons_protocol`, which
    /// nothing in this workspace depends on — so nothing can check the mirror
    /// holds. Pinning them here at least makes a change on this side a visible
    /// edit to a test that says what it is mirroring, rather than a one-line
    /// constant nobody notices.
    #[test]
    fn the_bucket_thresholds_are_pinned() {
        assert!((EASY_MEAN_CONFIDENCE - 0.75).abs() < f32::EPSILON);
        assert!((AMBIGUOUS_VARIABILITY - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn a_bucket_survives_the_round_trip_through_its_stored_name() {
        for bucket in [
            ConfidenceBucket::Easy,
            ConfidenceBucket::Ambiguous,
            ConfidenceBucket::Hard,
        ] {
            assert_eq!(ConfidenceBucket::parse(bucket.as_str()), Some(bucket));
        }
        assert_eq!(ConfidenceBucket::parse("uncertain"), None);
    }
}
