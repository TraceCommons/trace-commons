// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Units and posture for the credit-numbers endpoints.
//!
//! Pure: no I/O, no HTTP, no database. Both endpoints render their figures
//! through here so a contributor and a public reader can never be told two
//! different stories about the same deployment.

use serde::Serialize;

/// A deployment's configured points-to-currency rate.
///
/// Optional everywhere. A deployment that has not set one does not tell
/// contributors their work is worth money, which is the correct posture until
/// the graded pipeline leaves shadow mode.
#[derive(Debug, Clone)]
pub struct CreditRate {
    /// Points that make one unit of currency.
    pub points_per_unit: f64,
    /// ISO 4217 code, reported verbatim to clients.
    pub code: String,
}

/// The currency view of a points figure. Serialized only when it exists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CurrencyBlock {
    pub code: String,
    /// Fixed two places, as a string: a float would invite a client to do
    /// arithmetic on money the server already rounded.
    pub earned_this_period: String,
}

/// What this deployment is actually doing with credit.
#[derive(Debug, Clone, Serialize)]
pub struct CreditPosture {
    /// The live value of `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`.
    pub settlement: String,
    /// Whether quality, duplicate penalty and the per-contributor cap are
    /// authoritative. False while that pipeline is shadow-mode, which is what
    /// lets a client say a figure may still be revised.
    pub graded: bool,
    /// The same sentence the submission receipt gives, so two surfaces cannot
    /// describe one deployment differently.
    pub explanation: String,
}

impl CreditPosture {
    #[must_use]
    pub fn current(settlement_mode: &str, graded: bool) -> Self {
        Self {
            settlement: settlement_mode.to_string(),
            graded,
            explanation: settlement_posture_sentence(settlement_mode, graded),
        }
    }
}

/// The settlement-mode sentence, independent of grading status.
///
/// This is the single source for wording that `settlement_posture_explanation`
/// in the ingest binary and `CreditPosture` here both need to agree about:
/// #445 was a pilot that ran three months with 307 credit events stuck
/// `pending` because nothing told a contributor that `Disabled` settlement
/// makes the worker a no-op. Ingest maps its `NearSettlementMode` variant to
/// the labels below (`"http"`, `"dry_run"`, or anything else for the
/// fail-safe disabled case) and delegates here rather than holding its own
/// copy of this text -- two copies is how the wording drifted the first time.
#[must_use]
pub fn settlement_status_sentence(mode_label: &str) -> &'static str {
    match mode_label {
        "http" => "Credit is queued for on-chain settlement.",
        // The outbox advances with synthetic transaction hashes and no funds.
        // A settled-looking row here is not an on-chain credit.
        "dry_run" => {
            "Settlement is running in dry-run: the credit ledger advances with \
             synthetic transaction hashes and no on-chain credit is issued."
        }
        // Deliberate and fail-safe, not a fault. Say both halves: the credit
        // is real and recorded, and nothing is going to settle it here.
        _ => {
            "Credit is recorded but not settled: on-chain settlement is not \
             enabled on this deployment, so this figure stays pending."
        }
    }
}

/// One sentence describing the deployment's posture, for `CreditPosture`.
///
/// Composes `settlement_status_sentence` with the grading caveat: while the
/// quality/dedup/cap pipeline is shadow-mode, a figure may still be revised,
/// and this is the only place that says so.
fn settlement_posture_sentence(settlement_mode: &str, graded: bool) -> String {
    let settlement = settlement_status_sentence(settlement_mode);
    if graded {
        settlement.to_string()
    } else {
        format!(
            "{settlement} It is an estimate and may be revised as your \
             submissions are scored."
        )
    }
}

/// The currency view of `points`, or `None` when this deployment has no rate.
///
/// `None` is the whole point of the signature. A deployment without a rate has
/// made no claim about what a point is worth, and an absent key cannot be
/// misread the way a `0.00` can.
#[must_use]
pub fn currency_for(points: i64, rate: Option<&CreditRate>) -> Option<CurrencyBlock> {
    let rate = rate?;
    if !rate.points_per_unit.is_finite() || rate.points_per_unit <= 0.0 {
        return None;
    }
    let units = points as f64 / rate.points_per_unit;
    if !units.is_finite() {
        return None;
    }
    Some(CurrencyBlock {
        code: rate.code.clone(),
        earned_this_period: format!("{units:.2}"),
    })
}

/// Read the configured rate, or `None`.
///
/// Both variables are required together: a rate without a currency code names
/// no unit, and a code without a rate converts nothing.
#[must_use]
pub fn rate_from_env() -> Option<CreditRate> {
    let points_per_unit = std::env::var("TRACE_COMMONS_CREDIT_POINTS_PER_CURRENCY_UNIT")
        .ok()?
        .parse::<f64>()
        .ok()?;
    let code = std::env::var("TRACE_COMMONS_CREDIT_CURRENCY_CODE").ok()?;
    if code.trim().is_empty() {
        return None;
    }
    Some(CreditRate {
        points_per_unit,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_configured_rate_yields_no_currency_block_at_all() {
        // Absent, never zero. A client that sees no key shows points; a client
        // that saw `0.00` would tell a contributor they earned nothing.
        assert!(currency_for(1240, None).is_none());
    }

    #[test]
    fn a_configured_rate_converts_points_to_a_fixed_two_place_string() {
        let rate = CreditRate {
            points_per_unit: 100.0,
            code: "USD".to_string(),
        };
        let block = currency_for(1240, Some(&rate)).expect("a rate yields a block");
        assert_eq!(block.code, "USD");
        assert_eq!(block.earned_this_period, "12.40");
    }

    #[test]
    fn zero_points_with_a_rate_is_still_a_block_reading_zero() {
        // Distinct from the no-rate case above: here the contributor really
        // did earn nothing, and saying so is correct.
        let rate = CreditRate {
            points_per_unit: 100.0,
            code: "USD".to_string(),
        };
        let block = currency_for(0, Some(&rate)).expect("a rate yields a block");
        assert_eq!(block.earned_this_period, "0.00");
    }

    #[test]
    fn a_nonsense_rate_yields_no_currency_rather_than_a_wrong_number() {
        // Fail closed: a zero or negative divisor means the deployment is
        // misconfigured, and no figure is better than an invented one.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let rate = CreditRate {
                points_per_unit: bad,
                code: "USD".to_string(),
            };
            assert!(
                currency_for(1240, Some(&rate)).is_none(),
                "rate {bad} must not convert"
            );
        }
    }

    #[test]
    fn posture_reports_ungraded_while_the_pipeline_is_shadow_mode() {
        let posture = CreditPosture::current("disabled", false);
        assert_eq!(posture.settlement, "disabled");
        assert!(!posture.graded);
        assert!(
            !posture.explanation.is_empty(),
            "a posture always states itself in words"
        );
    }

    // #445: this text is the single copy the submission receipt
    // (`settlement_posture_explanation` in the ingest binary) delegates to
    // rather than holding its own. Pinning the dry-run sentence's full
    // wording here catches an edit to this copy; the ingest binary's own
    // `a_dry_run_receipt_does_not_imply_an_on_chain_credit` test catches a
    // wiring break on the delegating side, so the two together stand in for
    // an "agree" test that would otherwise be tautological now that there is
    // only one copy to compare against itself.
    #[test]
    fn dry_run_sentence_names_both_the_fake_hash_and_the_missing_credit() {
        let sentence = settlement_status_sentence("dry_run");
        assert_eq!(
            sentence,
            "Settlement is running in dry-run: the credit ledger advances with \
             synthetic transaction hashes and no on-chain credit is issued."
        );
    }

    #[test]
    fn unrecognized_labels_fail_safe_to_the_disabled_sentence() {
        // Mirrors NearSettlementMode::from_env(): unset, blank, or garbled
        // resolves to disabled, never to a settled-sounding default.
        assert_eq!(
            settlement_status_sentence("anything else"),
            settlement_status_sentence("disabled")
        );
    }
}
