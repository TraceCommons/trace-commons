// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust: the growth policy and the tiered rule. Pure: no
//! database, no clock. See
//! `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//!
//! [`parse_growth_policy`] parses a `tiered-v1` policy. A shadow candidate is
//! supplied separately from the admission policy and never raises an
//! allowance. When the admission policy itself names `tiered-v1`, the same
//! parser reads it, the worker stores `applied` evaluations, and admission
//! reads the latest fresh one (`account_trust::ExternalGrowth`).
//!
//! Attested inference (decided by Zaki on 2026-10-11, reversing decision 5 of
//! the spec): an accepted submission with a provider-TEE attested final call
//! qualifies without the gate and quality test, counts
//! `attested_unit_weight` units toward a tier, and a tier may require a
//! number of attested units. A policy may explicitly require a passed gate
//! even for attested submissions; the default preserves the attestation bypass.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::account_trust::{BoundedPolicy, PolicyError, parse_bounded_policy};

/// The only growth rule this build knows. `"none"` stays the production value.
pub const GROWTH_RULE_TIERED_V1: &str = "tiered-v1";
/// A tier table longer than this is refused; it is meant to be explained in
/// one sentence.
const MAX_TIERS: usize = 8;
const MAX_ALLOWLIST: usize = 64;
const WEEK_DAYS: i64 = 7;
const WEEK_SECONDS: i64 = WEEK_DAYS * 86_400;

/// One row of the tier table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthTier {
    pub units: u32,
    pub active_weeks: u32,
    pub age_seconds: i64,
    pub multiplier: i64,
    /// Attested units this tier requires, counted under the weekly cap.
    pub attested_units: u32,
}

/// A `tiered-v1` growth policy: a shadow candidate, or the admission
/// policy's own growth rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthPolicy {
    base: BoundedPolicy,
    window_seconds: i64,
    weekly_cap: u32,
    /// Units one attested qualified contribution counts toward a tier.
    attested_unit_weight: u32,
    /// Whether attestation must also satisfy the allowlisted passed gate.
    /// False preserves existing policies' attestation bypass.
    require_gate_pass: bool,
    /// `None` runs the rule on gate pass alone: decision 2 of the spec lets
    /// the first shadow run do that while credit-quality V3 settles.
    q_min_micros: Option<i64>,
    penalty_cooldown_seconds: i64,
    evaluation_max_age_seconds: i64,
    allowance_ceiling: i64,
    evaluator_versions: BTreeSet<String>,
    credit_quality_calibration_versions: BTreeSet<i32>,
    dedup_signal_versions: BTreeSet<String>,
    tiers: Vec<GrowthTier>,
}

impl GrowthPolicy {
    pub fn base(&self) -> &BoundedPolicy {
        &self.base
    }
    pub fn version(&self) -> &str {
        self.base.version()
    }
    pub fn evaluation_max_age_seconds(&self) -> i64 {
        self.evaluation_max_age_seconds
    }
    pub fn allowance_ceiling(&self) -> i64 {
        self.allowance_ceiling
    }
    pub fn tiers(&self) -> &[GrowthTier] {
        &self.tiers
    }
    pub fn attested_unit_weight(&self) -> u32 {
        self.attested_unit_weight
    }
    pub fn require_gate_pass(&self) -> bool {
        self.require_gate_pass
    }
    /// `min(bounded_allowance * m_tier, allowance_ceiling)`. The top tier's
    /// product is checked at parse time, so no tier can overflow.
    pub fn effective_allowance(&self, tier: u32) -> i64 {
        let multiplier = self
            .tiers
            .get(tier as usize)
            .map_or(1, |row| row.multiplier);
        self.base
            .bounded_allowance()
            .saturating_mul(multiplier)
            .min(self.allowance_ceiling)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrowth {
    window_seconds: i64,
    weekly_cap: u32,
    #[serde(default)]
    attested_unit_weight: Option<u32>,
    #[serde(default)]
    require_gate_pass: bool,
    q_min_micros: Option<i64>,
    penalty_cooldown_seconds: i64,
    evaluation_max_age_seconds: i64,
    allowance_ceiling: i64,
    evaluator_versions: Vec<String>,
    #[serde(default)]
    credit_quality_calibration_versions: Vec<i32>,
    dedup_signal_versions: Vec<String>,
    tiers: Vec<RawTier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTier {
    units: u32,
    active_weeks: u32,
    age_seconds: i64,
    multiplier: i64,
    #[serde(default)]
    attested_units: u32,
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'))
}

/// Parses a `tiered-v1` growth policy: the base policy fields exactly as
/// `parse_bounded_policy` takes them, plus `"growth_rule": "tiered-v1"` and a
/// `growth` object. The base fields are validated by `parse_bounded_policy`
/// itself (with the growth rule and object removed), so the two can never
/// disagree about what a valid base is. Used for the shadow candidate and,
/// through `parse_bounded_policy`, for an admission policy that names
/// `tiered-v1`.
pub fn parse_growth_policy(
    json: &str,
    supported_versions: &[&str],
) -> Result<GrowthPolicy, PolicyError> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(|_| PolicyError)?;
    let object = value.as_object_mut().ok_or(PolicyError)?;
    if object
        .get("growth_rule")
        .and_then(serde_json::Value::as_str)
        != Some(GROWTH_RULE_TIERED_V1)
    {
        return Err(PolicyError);
    }
    let growth = object.remove("growth").ok_or(PolicyError)?;
    object.insert("growth_rule".into(), "none".into());
    let base = parse_bounded_policy(&value.to_string(), supported_versions)?;
    let raw: RawGrowth = serde_json::from_value(growth).map_err(|_| PolicyError)?;
    // Absent is 1: an attested unit counts as much as any other. A weight is
    // capped with the week, so `weight * weekly_cap` must fit.
    let attested_unit_weight = raw.attested_unit_weight.unwrap_or(1);

    let quality_ok = match raw.q_min_micros {
        Some(q) => {
            (0..=1_000_000).contains(&q) && !raw.credit_quality_calibration_versions.is_empty()
        }
        None => raw.credit_quality_calibration_versions.is_empty(),
    };
    let labels_ok = |labels: &[String]| {
        !labels.is_empty() && labels.len() <= MAX_ALLOWLIST && labels.iter().all(|l| safe_label(l))
    };
    if raw.window_seconds <= 0
        || raw.weekly_cap == 0
        || attested_unit_weight == 0
        || attested_unit_weight.checked_mul(raw.weekly_cap).is_none()
        || !quality_ok
        || raw.credit_quality_calibration_versions.len() > MAX_ALLOWLIST
        || raw.penalty_cooldown_seconds < 0
        || raw.evaluation_max_age_seconds <= 0
        || raw.allowance_ceiling < base.bounded_allowance()
        || raw
            .allowance_ceiling
            .checked_add(base.processing_cost_bound())
            .is_none()
        || !labels_ok(&raw.evaluator_versions)
        || !labels_ok(&raw.dedup_signal_versions)
        || raw.tiers.is_empty()
        || raw.tiers.len() > MAX_TIERS
    {
        return Err(PolicyError);
    }
    let first = &raw.tiers[0];
    if first.units != 0
        || first.active_weeks != 0
        || first.age_seconds != 0
        || first.multiplier != 1
        || first.attested_units != 0
    {
        return Err(PolicyError);
    }
    if raw
        .tiers
        .iter()
        .any(|tier| tier.attested_units > tier.units)
    {
        return Err(PolicyError);
    }
    for pair in raw.tiers.windows(2) {
        let (lower, upper) = (&pair[0], &pair[1]);
        if upper.units < lower.units
            || upper.active_weeks < lower.active_weeks
            || upper.age_seconds < lower.age_seconds
            || upper.multiplier < lower.multiplier
            || upper.attested_units < lower.attested_units
        {
            return Err(PolicyError);
        }
    }
    let top = raw.tiers.last().map_or(1, |tier| tier.multiplier);
    if base.bounded_allowance().checked_mul(top).is_none() {
        return Err(PolicyError);
    }
    Ok(GrowthPolicy {
        base,
        window_seconds: raw.window_seconds,
        weekly_cap: raw.weekly_cap,
        attested_unit_weight,
        require_gate_pass: raw.require_gate_pass,
        q_min_micros: raw.q_min_micros,
        penalty_cooldown_seconds: raw.penalty_cooldown_seconds,
        evaluation_max_age_seconds: raw.evaluation_max_age_seconds,
        allowance_ceiling: raw.allowance_ceiling,
        evaluator_versions: raw.evaluator_versions.into_iter().collect(),
        credit_quality_calibration_versions: raw
            .credit_quality_calibration_versions
            .into_iter()
            .collect(),
        dedup_signal_versions: raw.dedup_signal_versions.into_iter().collect(),
        tiers: raw
            .tiers
            .into_iter()
            .map(|tier| GrowthTier {
                units: tier.units,
                active_weeks: tier.active_weeks,
                age_seconds: tier.age_seconds,
                multiplier: tier.multiplier,
                attested_units: tier.attested_units,
            })
            .collect(),
    })
}

/// One fact as the evaluator reads it: the fact row plus, for a gate
/// evaluation, the current gate-decision fields, and for a gate evaluation or
/// an attested submission the cross-tenant first-in-cluster boolean (V86,
/// V126). The IDs never leave the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationFact {
    pub source_kind: String,
    pub source_id: Uuid,
    pub submission_id: Uuid,
    pub outcome: String,
    pub evaluator_version: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub credit_quality_micros: Option<i64>,
    pub credit_quality_calibration_version: Option<i32>,
    pub dedup_signal_version: Option<String>,
    pub first_in_cluster: Option<bool>,
}

/// An evaluation: counts, a tier and a digest. No identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evaluation {
    pub growth_policy_version: String,
    pub as_of: DateTime<Utc>,
    pub tier: u32,
    pub effective_allowance: i64,
    pub units: u32,
    /// Units the weekly cap discarded: its binding rate. Weighted, as `units`.
    pub units_capped: u32,
    /// Attested qualified contributions, counted under the weekly cap. What a
    /// tier's `attested_units` requirement reads.
    pub attested_units: u32,
    pub active_weeks: u32,
    pub age_weeks: u32,
    /// Seconds since the first acceptance: what a tier's `age_seconds`
    /// requirement is compared with.
    pub age_seconds: i64,
    pub netted_withdrawn: u32,
    pub netted_revoked: u32,
    pub netted_quarantined: u32,
    pub penalty_active: bool,
    pub facts_digest: String,
}

/// The rule. A pure function of `(policy, facts with occurred_at <= as_of,
/// as_of)`; the order of `facts` does not matter.
pub fn evaluate(
    policy: &GrowthPolicy,
    facts: &[EvaluationFact],
    as_of: DateTime<Utc>,
) -> Evaluation {
    let facts: Vec<&EvaluationFact> = facts.iter().filter(|f| f.occurred_at <= as_of).collect();
    let facts_digest = facts_digest(&facts);

    // The latest penalty: tier 0 until its cooldown ends, and nothing
    // accepted at or before it counts again.
    let latest_penalty = facts
        .iter()
        .filter(|f| f.source_kind == "abuse_penalty" && f.outcome == "penalized")
        .map(|f| f.occurred_at)
        .max();
    let penalty_active = latest_penalty
        .is_some_and(|at| as_of < at + chrono::Duration::seconds(policy.penalty_cooldown_seconds));

    // Gate-required status evidence is authoritative about reviewer
    // independence for the whole UUID. A legacy credit fact for that same
    // submission cannot override a nonqualifying status classification.
    // Existing policies continue to ignore the new status source entirely.
    let status_exclusions: BTreeSet<Uuid> = if policy.require_gate_pass {
        facts
            .iter()
            .filter(|fact| {
                fact.source_kind == "accepted_submission_status"
                    && matches!(
                        fact.outcome.as_str(),
                        "accepted_self_reviewed"
                            | "accepted_approver_unknown"
                            | "accepted_status_inactive"
                    )
            })
            .map(|fact| fact.submission_id)
            .collect()
    } else {
        BTreeSet::new()
    };

    // Age: since the first acceptance. A floor, not a reward: it never decays.
    let first_accepted = facts
        .iter()
        .filter(|f| {
            accepted_fact_qualifies(policy, f) && !status_exclusions.contains(&f.submission_id)
        })
        .map(|f| f.occurred_at)
        .min();
    let age_seconds = first_accepted.map_or(0, |at| (as_of - at).num_seconds().max(0));

    let window_start = as_of - chrono::Duration::seconds(policy.window_seconds);
    // Per week: (plain units, attested units).
    let mut per_week: BTreeMap<i64, (u32, u32)> = BTreeMap::new();
    let (mut netted_withdrawn, mut netted_revoked, mut netted_quarantined) = (0u32, 0u32, 0u32);
    let mut accepted: BTreeMap<Uuid, DateTime<Utc>> = BTreeMap::new();
    for fact in &facts {
        if accepted_fact_qualifies(policy, fact) && !status_exclusions.contains(&fact.submission_id)
        {
            let entry = accepted
                .entry(fact.submission_id)
                .or_insert(fact.occurred_at);
            *entry = (*entry).min(fact.occurred_at);
        }
    }
    for (submission, accepted_at) in accepted {
        if accepted_at <= window_start || latest_penalty.is_some_and(|p| accepted_at <= p) {
            continue;
        }
        // A withdrawal, revocation or quarantine at or after acceptance
        // removes this unit and nothing else. The earliest one names it.
        let netting = facts
            .iter()
            .filter(|f| f.submission_id == submission && f.occurred_at >= accepted_at)
            .filter_map(|f| {
                let rank = match (f.source_kind.as_str(), f.outcome.as_str()) {
                    ("submission_withdrawn", "withdrawn") => 0,
                    ("submission_revoked", "revoked") => 1,
                    ("submission_quarantined", "quarantined") => 2,
                    _ => return None,
                };
                Some((f.occurred_at, rank))
            })
            .min();
        match netting {
            Some((_, 0)) => {
                netted_withdrawn += 1;
                continue;
            }
            Some((_, 1)) => {
                netted_revoked += 1;
                continue;
            }
            Some(_) => {
                netted_quarantined += 1;
                continue;
            }
            None => {}
        }
        // A provider-TEE attested final call stands in for the gate and
        // quality test unless the policy requires the passed gate too.
        // Acceptance, netting, the penalty cutoff and first-in-cluster apply
        // under either policy.
        let attested = facts
            .iter()
            .any(|f| f.submission_id == submission && attested_fact_qualifies(f));
        let gate_ok = facts
            .iter()
            .any(|f| f.submission_id == submission && gate_fact_qualifies(policy, f));
        if !gate_ok && (policy.require_gate_pass || !attested) {
            continue;
        }
        let week = crate::contributor_cap::epoch_index(accepted_at.timestamp(), WEEK_DAYS);
        let entry = per_week.entry(week).or_default();
        if attested {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    // The weight applies per unit and the cap to the weighted week, so
    // attestation buys volume no faster than the cap allows.
    let weighted = |(plain, attested): (u32, u32)| {
        plain.saturating_add(attested.saturating_mul(policy.attested_unit_weight))
    };
    let units: u32 = per_week
        .values()
        .map(|week| weighted(*week).min(policy.weekly_cap))
        .sum();
    let units_capped: u32 = per_week
        .values()
        .map(|week| weighted(*week).saturating_sub(policy.weekly_cap))
        .sum();
    let attested_units: u32 = per_week
        .values()
        .map(|(_, attested)| (*attested).min(policy.weekly_cap))
        .sum();
    let active_weeks = per_week
        .values()
        .filter(|week| weighted(**week) > 0)
        .count() as u32;

    let tier = if penalty_active {
        0
    } else {
        policy
            .tiers
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                units >= row.units
                    && active_weeks >= row.active_weeks
                    && age_seconds >= row.age_seconds
                    && attested_units >= row.attested_units
            })
            .map(|(index, _)| index as u32)
            .max()
            .unwrap_or(0)
    };
    Evaluation {
        growth_policy_version: policy.version().to_string(),
        as_of,
        tier,
        effective_allowance: policy.effective_allowance(tier),
        units,
        units_capped,
        attested_units,
        active_weeks,
        age_weeks: u32::try_from(age_seconds / WEEK_SECONDS).unwrap_or(u32::MAX),
        age_seconds,
        netted_withdrawn,
        netted_revoked,
        netted_quarantined,
        penalty_active,
        facts_digest,
    }
}

/// The V126 fact: a v2 witness certificate whose final call ran in a provider
/// TEE, matching the accepted artifact when recorded. A gateway receipt does
/// not show which model ran and is never recorded as this outcome.
fn accepted_fact_qualifies(policy: &GrowthPolicy, fact: &EvaluationFact) -> bool {
    fact.outcome == "accepted"
        && (fact.source_kind == "accepted_submission"
            || (policy.require_gate_pass && fact.source_kind == "accepted_submission_status"))
}

fn attested_fact_qualifies(fact: &EvaluationFact) -> bool {
    fact.source_kind == "attested_submission"
        && fact.outcome == "attested_provider_tee"
        && fact.first_in_cluster == Some(true)
}

fn gate_fact_qualifies(policy: &GrowthPolicy, fact: &EvaluationFact) -> bool {
    if fact.source_kind != "gate_evaluation" || fact.outcome != "evaluated_passed" {
        return false;
    }
    let evaluator_ok = fact
        .evaluator_version
        .as_ref()
        .is_some_and(|v| policy.evaluator_versions.contains(v));
    let quality_ok = match policy.q_min_micros {
        None => true,
        Some(q_min) => {
            fact.credit_quality_micros.is_some_and(|q| q >= q_min)
                && fact
                    .credit_quality_calibration_version
                    .is_some_and(|v| policy.credit_quality_calibration_versions.contains(&v))
        }
    };
    // NULL means "stamped before V57" and reads as the legacy v1 stamp, as
    // the dedup sweep reads it: never as unknown.
    let dedup_version = fact
        .dedup_signal_version
        .as_deref()
        .unwrap_or(crate::dedup_assign::LEGACY_DEDUP_SIGNAL_VERSION);
    evaluator_ok
        && quality_ok
        && policy.dedup_signal_versions.contains(dedup_version)
        && fact.first_in_cluster == Some(true)
}

/// `sha256` over the sorted, JSON-encoded tuples of every fact that entered
/// the evaluation, with the gate fields read for each. JSON arrays make the
/// encoding unambiguous; sorting makes it independent of row order.
fn facts_digest(facts: &[&EvaluationFact]) -> String {
    let mut tuples: Vec<String> = facts
        .iter()
        .map(|f| {
            serde_json::json!([
                f.source_kind,
                f.source_id,
                f.submission_id,
                f.outcome,
                f.evaluator_version,
                f.occurred_at.timestamp_micros(),
                f.credit_quality_micros,
                f.credit_quality_calibration_version,
                f.dedup_signal_version,
                f.first_in_cluster,
            ])
            .to_string()
        })
        .collect();
    tuples.sort();
    let mut hash = Sha256::new();
    hash.update(b"trace-account-trust-facts.v1\n");
    for tuple in tuples {
        hash.update(tuple.as_bytes());
        hash.update(b"\n");
    }
    format!("sha256:{}", hex::encode(hash.finalize()))
}

/// The head of a stored evaluation, as a reader of
/// `trace_account_trust_evaluations` sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredEvaluationHead<'a> {
    pub growth_policy_version: &'a str,
    pub mode: &'a str,
    pub as_of: DateTime<Utc>,
    pub tier: u32,
}

/// The tier admission may apply, per the spec's rule 2 ("How admission reads
/// trust") and the 2026-09-27 decisions: a missing evaluation, one computed
/// under a different policy version, a `shadow` one, a stale one (older than
/// `evaluation_max_age_seconds`), one dated in the future, or one naming a
/// tier the table does not have, all mean tier 0. Every failure falls to the
/// base allowance, never above it.
///
/// Not called by admission in this build: the production policy parser
/// refuses every growth rule, so admission never holds a [`GrowthPolicy`].
/// This is the reading the switch-on change wires in.
pub fn admission_tier(
    policy: &GrowthPolicy,
    stored: Option<StoredEvaluationHead<'_>>,
    now: DateTime<Utc>,
) -> u32 {
    applicable_tier(policy, stored, now, "applied")
}

/// The shadow counterpart of [`admission_tier`]: the same freshness and
/// version rules, reading a `shadow` row. Used only to count refusals the
/// candidate policy would have admitted; it never decides one.
pub fn shadow_tier(
    policy: &GrowthPolicy,
    stored: Option<StoredEvaluationHead<'_>>,
    now: DateTime<Utc>,
) -> u32 {
    applicable_tier(policy, stored, now, "shadow")
}

fn applicable_tier(
    policy: &GrowthPolicy,
    stored: Option<StoredEvaluationHead<'_>>,
    now: DateTime<Utc>,
    mode: &str,
) -> u32 {
    let Some(stored) = stored else {
        return 0;
    };
    let age = now - stored.as_of;
    if stored.growth_policy_version != policy.version()
        || stored.mode != mode
        || age < chrono::Duration::zero()
        || age > chrono::Duration::seconds(policy.evaluation_max_age_seconds)
        || stored.tier as usize >= policy.tiers.len()
    {
        return 0;
    }
    stored.tier
}

/// Whether a reservation refused as `account_limit_reached`, with
/// `period_spend` already spent, would have fitted under the candidate
/// policy's allowance at the account's shadow tier. A calibration count for
/// the switch-on decision, never an admission decision.
pub fn shadow_would_admit(
    policy: &GrowthPolicy,
    stored: Option<StoredEvaluationHead<'_>>,
    period_spend: i64,
    now: DateTime<Utc>,
) -> bool {
    let tier = shadow_tier(policy, stored, now);
    tier > 0
        && period_spend
            .checked_add(policy.base.processing_cost_bound())
            .is_some_and(|next| next <= policy.effective_allowance(tier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn admission_reads_only_a_fresh_applied_evaluation_of_its_own_version() {
        let policy = policy();
        let now = t(10);
        let head = |version: &'static str, mode: &'static str, age: i64, tier: u32| {
            Some(StoredEvaluationHead {
                growth_policy_version: version,
                mode,
                as_of: now - Duration::seconds(age),
                tier,
            })
        };
        assert_eq!(admission_tier(&policy, None, now), 0, "missing");
        assert_eq!(
            admission_tier(&policy, head("growth-test-v1", "applied", 60, 2), now),
            2
        );
        assert_eq!(
            admission_tier(&policy, head("growth-test-v0", "applied", 60, 2), now),
            0,
            "wrong version"
        );
        assert_eq!(
            admission_tier(&policy, head("growth-test-v1", "shadow", 60, 2), now),
            0,
            "a shadow row never raises an allowance"
        );
        assert_eq!(
            admission_tier(
                &policy,
                head("growth-test-v1", "applied", 2 * DAY + 1, 2),
                now
            ),
            0,
            "stale"
        );
        assert_eq!(
            admission_tier(&policy, head("growth-test-v1", "applied", 2 * DAY, 2), now),
            2,
            "exactly at the max age is still fresh"
        );
        assert_eq!(
            admission_tier(&policy, head("growth-test-v1", "applied", -1, 2), now),
            0,
            "dated in the future"
        );
        assert_eq!(
            admission_tier(&policy, head("growth-test-v1", "applied", 60, 3), now),
            0,
            "a tier the table does not have"
        );
    }

    #[test]
    fn a_shadow_refusal_is_counted_only_when_the_candidate_would_admit() {
        let policy = policy();
        let now = t(10);
        let shadow = |tier: u32| {
            Some(StoredEvaluationHead {
                growth_policy_version: "growth-test-v1",
                mode: "shadow",
                as_of: now - Duration::seconds(60),
                tier,
            })
        };
        // Base 100, cost bound 10; tier 1 allows 200, tier 2 the 400 ceiling.
        assert!(shadow_would_admit(&policy, shadow(1), 95, now));
        assert!(shadow_would_admit(&policy, shadow(1), 190, now));
        assert!(!shadow_would_admit(&policy, shadow(1), 191, now));
        assert!(shadow_would_admit(&policy, shadow(2), 390, now));
        assert!(
            !shadow_would_admit(&policy, shadow(0), 50, now),
            "tier 0 adds nothing"
        );
        assert!(!shadow_would_admit(&policy, None, 50, now));
        let applied = Some(StoredEvaluationHead {
            growth_policy_version: "growth-test-v1",
            mode: "applied",
            as_of: now,
            tier: 2,
        });
        assert!(
            !shadow_would_admit(&policy, applied, 50, now),
            "the shadow count reads shadow rows only"
        );
    }

    const DAY: i64 = 86_400;

    fn policy_json(growth: serde_json::Value) -> String {
        serde_json::json!({
            "version": "growth-test-v1",
            "processing_cost_bound": 10,
            "bounded_allowance": 100,
            "period": {"mode": "fixed", "seconds": 604800},
            "growth_rule": "tiered-v1",
            "growth": growth,
        })
        .to_string()
    }

    fn growth() -> serde_json::Value {
        serde_json::json!({
            "window_seconds": 28 * DAY,
            "weekly_cap": 3,
            "q_min_micros": 500_000,
            "penalty_cooldown_seconds": 14 * DAY,
            "evaluation_max_age_seconds": 2 * DAY,
            "allowance_ceiling": 400,
            "evaluator_versions": ["gate-v1"],
            "credit_quality_calibration_versions": [3],
            "dedup_signal_versions": ["events.v2+simhash.v2"],
            "tiers": [
                {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                {"units": 3, "active_weeks": 2, "age_seconds": 7 * DAY, "multiplier": 2},
                {"units": 6, "active_weeks": 3, "age_seconds": 21 * DAY, "multiplier": 5},
            ],
        })
    }

    fn policy() -> GrowthPolicy {
        parse_growth_policy(&policy_json(growth()), &["growth-test-v1"]).expect("valid")
    }

    fn with(mut value: serde_json::Value, key: &str, replacement: serde_json::Value) -> String {
        value[key] = replacement;
        policy_json(value)
    }

    fn t(day: i64) -> DateTime<Utc> {
        // A Thursday-aligned origin is irrelevant: weeks are 7-day buckets
        // from the Unix epoch, as contributor_cap::epoch_index counts them.
        Utc.timestamp_opt(20_000 * DAY + day * DAY, 0).unwrap()
    }

    struct Facts(Vec<EvaluationFact>);

    impl Facts {
        fn new() -> Self {
            Self(Vec::new())
        }
        fn push(&mut self, kind: &str, submission: Uuid, outcome: &str, at: DateTime<Utc>) {
            self.0.push(EvaluationFact {
                source_kind: kind.into(),
                source_id: if kind == "gate_evaluation" || kind == "abuse_penalty" {
                    Uuid::new_v4()
                } else {
                    submission
                },
                submission_id: submission,
                outcome: outcome.into(),
                evaluator_version: (kind == "gate_evaluation").then(|| "gate-v1".into()),
                occurred_at: at,
                credit_quality_micros: (kind == "gate_evaluation").then_some(900_000),
                credit_quality_calibration_version: (kind == "gate_evaluation").then_some(3),
                dedup_signal_version: (kind == "gate_evaluation")
                    .then(|| "events.v2+simhash.v2".into()),
                first_in_cluster: matches!(kind, "gate_evaluation" | "attested_submission")
                    .then_some(true),
            });
        }
        /// An accepted, first-in-cluster submission with a provider-TEE
        /// attestation and no gate evaluation at all, accepted on `day`.
        fn attested(&mut self, day: i64) -> Uuid {
            let submission = Uuid::new_v4();
            self.push("accepted_submission", submission, "accepted", t(day));
            self.push(
                "attested_submission",
                submission,
                "attested_provider_tee",
                t(day),
            );
            submission
        }
        fn last_attested(&mut self) -> &mut EvaluationFact {
            self.0
                .iter_mut()
                .rev()
                .find(|f| f.source_kind == "attested_submission")
                .unwrap()
        }
        /// An accepted, gate-passed, quality-clearing, first-in-cluster
        /// submission accepted on `day`.
        fn qualified(&mut self, day: i64) -> Uuid {
            let submission = Uuid::new_v4();
            self.push("accepted_submission", submission, "accepted", t(day));
            self.push("gate_evaluation", submission, "evaluated_passed", t(day));
            submission
        }
        fn last_gate(&mut self) -> &mut EvaluationFact {
            self.0
                .iter_mut()
                .rev()
                .find(|f| f.source_kind == "gate_evaluation")
                .unwrap()
        }
    }

    fn staging_upload_policy() -> GrowthPolicy {
        let json =
            include_str!("../../../deploy/staging-gcp/client-upload-trust-policy.json.template")
                .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
                .replace(
                    "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
                    "events.v2+simhash.v2",
                );
        parse_growth_policy(&json, &["staging-earned-upload-v1"]).unwrap()
    }

    #[test]
    fn status_acceptance_is_opt_in_gate_qualified_and_counted_once_per_submission() {
        let strict = staging_upload_policy();
        let mut facts = Facts::new();
        let submission = facts.qualified(1);
        facts
            .0
            .iter_mut()
            .find(|fact| fact.source_kind == "accepted_submission")
            .unwrap()
            .source_kind = "accepted_submission_status".into();
        assert_eq!(
            evaluate(&policy(), &facts.0, t(2)).units,
            0,
            "legacy policies never count a status-only acceptance"
        );
        assert_eq!(evaluate(&strict, &facts.0, t(2)).units, 1);
        for outcome in ["accepted_self_reviewed", "accepted_approver_unknown"] {
            facts
                .0
                .iter_mut()
                .find(|fact| fact.source_kind == "accepted_submission_status")
                .unwrap()
                .outcome = outcome.into();
            assert_eq!(
                evaluate(&strict, &facts.0, t(2)).units,
                0,
                "a passed gate does not override reviewer independence: {outcome}"
            );
        }
        facts
            .0
            .iter_mut()
            .find(|fact| fact.source_kind == "accepted_submission_status")
            .unwrap()
            .outcome = "accepted".into();
        facts.push("accepted_submission", submission, "accepted", t(1));
        assert_eq!(
            evaluate(&strict, &facts.0, t(2)).units,
            1,
            "credit and status sources for one UUID earn one unit"
        );
        for outcome in [
            "accepted_self_reviewed",
            "accepted_approver_unknown",
            "accepted_status_inactive",
        ] {
            facts
                .0
                .iter_mut()
                .find(|fact| fact.source_kind == "accepted_submission_status")
                .unwrap()
                .outcome = outcome.into();
            assert_eq!(
                evaluate(&strict, &facts.0, t(2)).units,
                0,
                "legacy accepted credit for the same UUID cannot override {outcome}"
            );
            assert_eq!(
                evaluate(&policy(), &facts.0, t(2)).units,
                1,
                "default-false policies preserve their legacy credit source behavior"
            );
        }
        facts
            .0
            .iter_mut()
            .find(|fact| fact.source_kind == "accepted_submission_status")
            .unwrap()
            .outcome = "accepted".into();
        facts
            .0
            .retain(|fact| fact.source_kind != "accepted_submission");
        facts.last_gate().outcome = "evaluated_failed".into();
        assert_eq!(evaluate(&strict, &facts.0, t(2)).units, 0);
        facts.last_gate().outcome = "evaluated_passed".into();
        for (kind, outcome) in [
            ("submission_withdrawn", "withdrawn"),
            ("submission_revoked", "revoked"),
            ("submission_quarantined", "quarantined"),
        ] {
            let mut netted = facts.0.clone();
            let mut adverse = Facts::new();
            adverse.push(kind, submission, outcome, t(2));
            netted.extend(adverse.0);
            assert_eq!(evaluate(&strict, &netted, t(3)).units, 0, "{kind}");
        }
    }

    #[test]
    fn staging_upload_tiers_have_exact_boundaries_without_age_or_week_waits() {
        let policy = staging_upload_policy();
        assert!(policy.require_gate_pass());
        assert_eq!(policy.attested_unit_weight(), 1);
        assert!(policy.tiers().iter().all(|tier| tier.active_weeks == 0
            && tier.age_seconds == 0
            && tier.attested_units == 0));
        for (units, tier, allowance) in [
            (0, 0, 100),
            (4, 0, 100),
            (5, 1, 200),
            (19, 1, 200),
            (20, 2, 300),
            (49, 2, 300),
            (50, 3, 500),
        ] {
            let mut facts = Facts::new();
            for _ in 0..units {
                facts.qualified(1);
            }
            // Every contribution is from this instant and the same week.
            let result = evaluate(&policy, &facts.0, t(1));
            assert_eq!(result.units, units);
            assert_eq!(result.age_seconds, 0);
            assert_eq!(result.active_weeks, u32::from(units > 0));
            assert_eq!(result.tier, tier, "{units} qualified traces");
            assert_eq!(result.effective_allowance, allowance);
        }
    }

    #[test]
    fn staging_upload_trust_counts_existing_qualified_history_in_its_window() {
        let policy = staging_upload_policy();
        let mut facts = Facts::new();
        for _ in 0..50 {
            facts.qualified(1);
        }
        assert_eq!(evaluate(&policy, &facts.0, t(84)).tier, 3);
        assert_eq!(
            evaluate(&policy, &facts.0, t(85)).units,
            0,
            "history outside the 84-day window expires"
        );
    }

    #[test]
    fn staging_upload_trust_excludes_duplicates_reuploads_and_netted_traces() {
        let policy = staging_upload_policy();
        let mut facts = Facts::new();
        for _ in 0..5 {
            facts.qualified(1);
        }
        // Repeated facts for an existing submission do not earn extra units.
        facts.0.extend(facts.0.clone());
        assert_eq!(evaluate(&policy, &facts.0, t(1)).units, 5);
        // A distinct re-upload in the same cluster cannot earn another unit.
        let duplicate = facts.qualified(1);
        facts.last_gate().first_in_cluster = Some(false);
        facts.push(
            "attested_submission",
            duplicate,
            "attested_provider_tee",
            t(1),
        );
        facts.last_attested().first_in_cluster = Some(false);
        assert_eq!(evaluate(&policy, &facts.0, t(1)).units, 5);
        let submissions: Vec<_> = facts
            .0
            .iter()
            .filter(|fact| fact.source_kind == "accepted_submission")
            .take(3)
            .map(|fact| fact.submission_id)
            .collect();
        for (submission, (kind, outcome)) in submissions.into_iter().zip([
            ("submission_withdrawn", "withdrawn"),
            ("submission_revoked", "revoked"),
            ("submission_quarantined", "quarantined"),
        ]) {
            facts.push(kind, submission, outcome, t(1));
        }
        let result = evaluate(&policy, &facts.0, t(1));
        assert_eq!((result.units, result.tier), (2, 0));
        assert_eq!(
            (
                result.netted_withdrawn,
                result.netted_revoked,
                result.netted_quarantined
            ),
            (1, 1, 1)
        );
    }

    #[test]
    fn gate_required_upload_trust_never_accepts_attestation_as_a_gate_bypass() {
        let policy = staging_upload_policy();
        let mut facts = Facts::new();
        let submission = facts.attested(1);
        assert_eq!(evaluate(&policy, &facts.0, t(1)).units, 0, "no gate");
        facts.push("gate_evaluation", submission, "evaluated_failed", t(1));
        assert_eq!(evaluate(&policy, &facts.0, t(1)).units, 0, "failed gate");
        facts.last_gate().outcome = "evaluated_passed".into();
        facts.last_gate().evaluator_version = Some("other-gate".into());
        assert_eq!(
            evaluate(&policy, &facts.0, t(1)).units,
            0,
            "unlisted gate version"
        );
        facts.last_gate().evaluator_version = Some("gate-v1".into());
        facts.last_gate().dedup_signal_version = Some("other-dedup".into());
        assert_eq!(
            evaluate(&policy, &facts.0, t(1)).units,
            0,
            "unlisted dedup version"
        );
        facts.last_gate().dedup_signal_version = Some("events.v2+simhash.v2".into());
        facts.last_gate().first_in_cluster = Some(false);
        assert_eq!(
            evaluate(&policy, &facts.0, t(1)).units,
            0,
            "duplicate even with first-cluster attestation"
        );
        facts.last_gate().first_in_cluster = Some(true);
        let result = evaluate(&policy, &facts.0, t(1));
        assert_eq!(
            (result.units, result.attested_units),
            (1, 1),
            "both pass, no bonus"
        );
    }

    #[test]
    fn requiring_gate_pass_defaults_to_false_for_existing_growth_policies() {
        let legacy = policy();
        assert!(!legacy.require_gate_pass());
        let explicit = parse_growth_policy(
            &with(growth(), "require_gate_pass", false.into()),
            &["growth-test-v1"],
        )
        .unwrap();
        assert_eq!(legacy, explicit);
        let mut facts = Facts::new();
        facts.attested(1);
        assert_eq!(evaluate(&legacy, &facts.0, t(2)).units, 1);
        assert!(
            parse_growth_policy(
                &with(growth(), "require_gate_pass", "true".into()),
                &["growth-test-v1"]
            )
            .is_err()
        );
    }

    // ---- parser -----------------------------------------------------------

    #[test]
    fn none_parses_only_through_the_admission_parser() {
        let none = r#"{"version":"growth-test-v1","processing_cost_bound":10,"bounded_allowance":100,"period":{"mode":"lifetime"},"growth_rule":"none"}"#;
        assert!(parse_bounded_policy(none, &["growth-test-v1"]).is_ok());
        assert!(
            parse_growth_policy(none, &["growth-test-v1"]).is_err(),
            "a shadow policy must name a growth rule"
        );
    }

    #[test]
    fn the_admission_parser_reads_tiered_v1_as_an_applied_growth_rule() {
        let admission = parse_bounded_policy(&policy_json(growth()), &["growth-test-v1"])
            .expect("tiered-v1 is an admission growth rule");
        let applied = admission.external_growth().expect("growth is read");
        assert_eq!(applied.policy_version, "growth-test-v1");
        assert_eq!(applied.allowance_ceiling, 400);
        assert_eq!(applied.evaluation_max_age_seconds, 2 * DAY);
        assert_eq!(applied.source, crate::account_trust::GrowthSource::TieredV1);
        assert_eq!(admission.bounded_allowance(), 100);
        // The growth object is validated by the same parser as a shadow one.
        let mut bad = growth();
        bad["weekly_cap"] = serde_json::json!(0);
        assert!(parse_bounded_policy(&policy_json(bad), &["growth-test-v1"]).is_err());
        // tiered-v1 takes no external fields, and neither none nor external
        // takes a growth object.
        let mut mixed: serde_json::Value = serde_json::from_str(&policy_json(growth())).unwrap();
        mixed["allowance_ceiling"] = serde_json::json!(400);
        assert!(parse_bounded_policy(&mixed.to_string(), &["growth-test-v1"]).is_err());
        let mut none_with_growth: serde_json::Value =
            serde_json::from_str(&policy_json(growth())).unwrap();
        none_with_growth["growth_rule"] = serde_json::json!("none");
        assert!(parse_bounded_policy(&none_with_growth.to_string(), &["growth-test-v1"]).is_err());
        let mut external_with_growth = none_with_growth.clone();
        external_with_growth["growth_rule"] = serde_json::json!("external");
        external_with_growth["growth_policy_version"] = serde_json::json!("g");
        external_with_growth["allowance_ceiling"] = serde_json::json!(400);
        external_with_growth["evaluation_max_age_seconds"] = serde_json::json!(60);
        assert!(
            parse_bounded_policy(&external_with_growth.to_string(), &["growth-test-v1"]).is_err()
        );
        // none is unchanged: no growth read at all.
        let none = r#"{"version":"growth-test-v1","processing_cost_bound":10,"bounded_allowance":100,"period":{"mode":"lifetime"},"growth_rule":"none"}"#;
        assert_eq!(
            parse_bounded_policy(none, &["growth-test-v1"])
                .unwrap()
                .external_growth(),
            None
        );
    }

    #[test]
    fn a_valid_policy_parses_and_prices_each_tier_under_the_ceiling() {
        let policy = policy();
        assert_eq!(policy.version(), "growth-test-v1");
        assert_eq!(policy.tiers().len(), 3);
        assert_eq!(policy.effective_allowance(0), 100);
        assert_eq!(policy.effective_allowance(1), 200);
        assert_eq!(
            policy.effective_allowance(2),
            400,
            "5x is capped at the ceiling"
        );
        let gate_only = with(growth(), "q_min_micros", serde_json::Value::Null);
        let mut gate_only: serde_json::Value = serde_json::from_str(&gate_only).unwrap();
        gate_only["growth"]["credit_quality_calibration_versions"] = serde_json::json!([]);
        assert!(
            parse_growth_policy(&gate_only.to_string(), &["growth-test-v1"]).is_ok(),
            "decision 2: gate pass alone, with no quality era named"
        );
    }

    #[test]
    fn every_malformed_growth_object_is_refused() {
        let v = &["growth-test-v1"];
        let tiers = |t: serde_json::Value| with(growth(), "tiers", t);
        let refused = [
            (
                "unknown rule",
                policy_json(growth()).replace("tiered-v1", "tiered-v2"),
            ),
            ("missing growth", {
                let mut j: serde_json::Value =
                    serde_json::from_str(&policy_json(growth())).unwrap();
                j.as_object_mut().unwrap().remove("growth");
                j.to_string()
            }),
            ("unknown field", {
                let mut g = growth();
                g["surprise"] = serde_json::json!(1);
                policy_json(g)
            }),
            (
                "unsupported version",
                policy_json(growth()).replace("growth-test-v1", "other-v1"),
            ),
            (
                "zero window",
                with(growth(), "window_seconds", serde_json::json!(0)),
            ),
            (
                "zero weekly cap",
                with(growth(), "weekly_cap", serde_json::json!(0)),
            ),
            (
                "negative q",
                with(growth(), "q_min_micros", serde_json::json!(-1)),
            ),
            (
                "q above one",
                with(growth(), "q_min_micros", serde_json::json!(1_000_001)),
            ),
            (
                "negative cooldown",
                with(growth(), "penalty_cooldown_seconds", serde_json::json!(-1)),
            ),
            (
                "zero max age",
                with(growth(), "evaluation_max_age_seconds", serde_json::json!(0)),
            ),
            (
                "ceiling below base",
                with(growth(), "allowance_ceiling", serde_json::json!(99)),
            ),
            (
                "empty evaluators",
                with(growth(), "evaluator_versions", serde_json::json!([])),
            ),
            (
                "empty dedup",
                with(growth(), "dedup_signal_versions", serde_json::json!([])),
            ),
            (
                "bad label",
                with(
                    growth(),
                    "evaluator_versions",
                    serde_json::json!(["gate v1"]),
                ),
            ),
            (
                "q with no quality era",
                with(
                    growth(),
                    "credit_quality_calibration_versions",
                    serde_json::json!([]),
                ),
            ),
            ("no tiers", tiers(serde_json::json!([]))),
            (
                "tier 0 not zero",
                tiers(
                    serde_json::json!([{"units": 1, "active_weeks": 0, "age_seconds": 0, "multiplier": 1}]),
                ),
            ),
            (
                "tier 0 multiplier",
                tiers(
                    serde_json::json!([{"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 2}]),
                ),
            ),
            (
                "non-monotone units",
                tiers(serde_json::json!([
                    {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                    {"units": 5, "active_weeks": 1, "age_seconds": 0, "multiplier": 2},
                    {"units": 4, "active_weeks": 2, "age_seconds": 0, "multiplier": 3}
                ])),
            ),
            (
                "non-monotone multiplier",
                tiers(serde_json::json!([
                    {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                    {"units": 5, "active_weeks": 1, "age_seconds": 0, "multiplier": 3},
                    {"units": 6, "active_weeks": 2, "age_seconds": 0, "multiplier": 2}
                ])),
            ),
            (
                "overflow at the top tier",
                tiers(serde_json::json!([
                    {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                    {"units": 1, "active_weeks": 1, "age_seconds": 0, "multiplier": i64::MAX}
                ])),
            ),
        ];
        for (label, json) in refused {
            assert!(parse_growth_policy(&json, v).is_err(), "{label}");
        }
    }

    // ---- rule -------------------------------------------------------------

    #[test]
    fn no_facts_is_tier_zero_at_the_base_allowance() {
        let e = evaluate(&policy(), &[], t(100));
        assert_eq!((e.tier, e.units, e.active_weeks, e.age_weeks), (0, 0, 0, 0));
        assert_eq!(e.effective_allowance, 100);
        assert!(!e.penalty_active);
        assert!(e.facts_digest.starts_with("sha256:"));
    }

    #[test]
    fn qualified_contributions_climb_the_tiers_with_calendar_time() {
        let mut f = Facts::new();
        // Three weeks of three units each; the account is 21 days old at day 21.
        for week in 0..3 {
            for _ in 0..3 {
                f.qualified(week * 7 + 1);
            }
        }
        let e = evaluate(&policy(), &f.0, t(22));
        assert_eq!(e.units, 9);
        assert_eq!(e.active_weeks, 3);
        assert_eq!(e.age_weeks, 3);
        assert_eq!(e.tier, 2);
        assert_eq!(e.effective_allowance, 400);
        // The same history a week earlier is too young for tier 2.
        let earlier = evaluate(&policy(), &f.0, t(15));
        assert_eq!(earlier.tier, 1, "units alone do not buy the upper tiers");
    }

    #[test]
    fn the_weekly_cap_limits_units_per_week_and_is_reported() {
        let mut f = Facts::new();
        for _ in 0..10 {
            f.qualified(1);
        }
        let e = evaluate(&policy(), &f.0, t(2));
        assert_eq!(e.units, 3, "at most weekly_cap units in one week");
        assert_eq!(e.units_capped, 7);
        assert_eq!(e.active_weeks, 1);
    }

    #[test]
    fn the_window_drops_old_units_but_age_does_not_decay() {
        let mut f = Facts::new();
        for week in 0..3 {
            for _ in 0..3 {
                f.qualified(week * 7 + 1);
            }
        }
        // Window is 28 days: at day 60 every unit has aged out.
        let e = evaluate(&policy(), &f.0, t(60));
        assert_eq!((e.units, e.active_weeks, e.tier), (0, 0, 0));
        assert_eq!(e.age_weeks, 8, "age is a floor, not a reward");
        // Window edge: a unit exactly W old has left; one second younger has not.
        let mut edge = Facts::new();
        edge.qualified(0);
        let at_edge = t(0) + Duration::seconds(28 * DAY);
        assert_eq!(evaluate(&policy(), &edge.0, at_edge).units, 0);
        assert_eq!(
            evaluate(&policy(), &edge.0, at_edge - Duration::seconds(1)).units,
            1
        );
        // A fact after as_of is not read at all.
        let mut future = Facts::new();
        future.qualified(10);
        let before = evaluate(&policy(), &future.0, t(9));
        assert_eq!((before.units, before.age_weeks), (0, 0));
    }

    #[test]
    fn each_requirement_of_a_qualified_contribution_is_checked() {
        let base = |mutate: &dyn Fn(&mut Facts)| {
            let mut f = Facts::new();
            f.qualified(1);
            mutate(&mut f);
            evaluate(&policy(), &f.0, t(2)).units
        };
        assert_eq!(base(&|_| {}), 1);
        assert_eq!(
            base(&|f| f.0.retain(|x| x.source_kind != "accepted_submission")),
            0
        );
        assert_eq!(
            base(&|f| f.0.retain(|x| x.source_kind != "gate_evaluation")),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().outcome = "evaluated_failed".into()),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().outcome = "evaluated_not_accepted".into()),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().evaluator_version = Some("gate-v0".into())),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().credit_quality_micros = Some(499_999)),
            0
        );
        assert_eq!(base(&|f| f.last_gate().credit_quality_micros = None), 0);
        assert_eq!(
            base(&|f| f.last_gate().credit_quality_calibration_version = Some(2)),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().dedup_signal_version = Some("digest-prefix.v1".into())),
            0
        );
        assert_eq!(
            base(&|f| f.last_gate().dedup_signal_version = None),
            0,
            "NULL reads as the legacy v1 stamp, which is not allowlisted"
        );
        assert_eq!(base(&|f| f.last_gate().first_in_cluster = Some(false)), 0);
        assert_eq!(base(&|f| f.last_gate().first_in_cluster = None), 0);
        assert_eq!(
            base(&|f| {
                let s = f.0[0].submission_id;
                f.0[0].outcome = "accepted_self_reviewed".into();
                let _ = s;
            }),
            0
        );
        assert_eq!(
            base(&|f| f.0[0].outcome = "accepted_approver_unknown".into()),
            0
        );
        // One passing decision among several is enough; duplicates count once.
        assert_eq!(
            base(&|f| {
                let s = f.0[0].submission_id;
                f.push("gate_evaluation", s, "evaluated_failed", t(1));
                f.push("gate_evaluation", s, "evaluated_passed", t(1));
            }),
            1
        );
    }

    #[test]
    fn gate_pass_alone_counts_when_no_quality_threshold_is_set() {
        let mut g = growth();
        g["q_min_micros"] = serde_json::Value::Null;
        g["credit_quality_calibration_versions"] = serde_json::json!([]);
        let policy = parse_growth_policy(&policy_json(g), &["growth-test-v1"]).unwrap();
        let mut f = Facts::new();
        f.qualified(1);
        f.last_gate().credit_quality_micros = None;
        f.last_gate().credit_quality_calibration_version = None;
        assert_eq!(evaluate(&policy, &f.0, t(2)).units, 1);
    }

    #[test]
    fn later_withdrawal_revocation_or_quarantine_nets_a_unit_out_without_penalty() {
        for (kind, outcome) in [
            ("submission_withdrawn", "withdrawn"),
            ("submission_revoked", "revoked"),
            ("submission_quarantined", "quarantined"),
        ] {
            let mut f = Facts::new();
            let keep = f.qualified(1);
            let _ = keep;
            let gone = f.qualified(2);
            f.push(kind, gone, outcome, t(3));
            let e = evaluate(&policy(), &f.0, t(4));
            assert_eq!(e.units, 1, "{kind} removes exactly its own unit");
            assert!(!e.penalty_active, "{kind} is never punitive");
            let netted = (e.netted_withdrawn, e.netted_revoked, e.netted_quarantined);
            let expected = match outcome {
                "withdrawn" => (1, 0, 0),
                "revoked" => (0, 1, 0),
                _ => (0, 0, 1),
            };
            assert_eq!(netted, expected);
            // Before the netting fact happened, the unit still counted.
            assert_eq!(
                evaluate(&policy(), &f.0, t(2) + Duration::hours(1)).units,
                2
            );
        }
        // A quarantine that happened before acceptance (then remediated) does
        // not net the later acceptance out.
        let mut f = Facts::new();
        let s = Uuid::new_v4();
        f.push("submission_quarantined", s, "quarantined", t(1));
        f.push("accepted_submission", s, "accepted", t(2));
        f.push("gate_evaluation", s, "evaluated_passed", t(2));
        assert_eq!(evaluate(&policy(), &f.0, t(3)).units, 1);
    }

    #[test]
    fn a_penalty_sets_tier_zero_for_the_cooldown_and_discards_earlier_units() {
        let mut f = Facts::new();
        for week in 0..3 {
            for _ in 0..3 {
                f.qualified(week * 7 + 1);
            }
        }
        assert_eq!(evaluate(&policy(), &f.0, t(22)).tier, 2);
        let s = f.0[0].submission_id;
        f.push("abuse_penalty", s, "penalized", t(22) + Duration::hours(1));
        let during = evaluate(&policy(), &f.0, t(23));
        assert!(during.penalty_active);
        assert_eq!(during.tier, 0);
        assert_eq!(during.effective_allowance, 100);
        assert_eq!(during.units, 0, "units before the penalty stop counting");
        // After the 14-day cooldown the account re-earns from later facts only.
        let after = evaluate(&policy(), &f.0, t(40));
        assert!(!after.penalty_active);
        assert_eq!((after.units, after.tier), (0, 0));
        for week in 0..3 {
            for _ in 0..3 {
                f.qualified(40 + week * 7);
            }
        }
        let rebuilt = evaluate(&policy(), &f.0, t(55));
        assert_eq!(rebuilt.units, 9);
        assert_eq!(rebuilt.tier, 2);
    }

    #[test]
    fn the_digest_is_order_independent_and_changes_with_any_field() {
        let mut f = Facts::new();
        for day in 0..6 {
            f.qualified(day);
        }
        let s = f.0[0].submission_id;
        f.push("submission_withdrawn", s, "withdrawn", t(6));
        let forward = evaluate(&policy(), &f.0, t(7));
        let mut reversed = f.0.clone();
        reversed.reverse();
        assert_eq!(forward, evaluate(&policy(), &reversed, t(7)));
        let mut changed = f.0.clone();
        changed[1].credit_quality_micros = Some(900_001);
        assert_ne!(
            forward.facts_digest,
            evaluate(&policy(), &changed, t(7)).facts_digest,
            "the gate fields read for a fact are part of the digest"
        );
        let mut cluster = f.0.clone();
        cluster[1].first_in_cluster = Some(false);
        assert_ne!(
            forward.facts_digest,
            evaluate(&policy(), &cluster, t(7)).facts_digest
        );
        // Facts after as_of are not in the digest.
        let mut later = f.0.clone();
        later.push(EvaluationFact {
            occurred_at: t(30),
            ..f.0[0].clone()
        });
        assert_eq!(
            forward.facts_digest,
            evaluate(&policy(), &later, t(7)).facts_digest
        );
    }

    // ---- attested inference (decided 2026-10-11) --------------------------

    fn attested_policy(weight: Option<u32>, tier_attested: [u32; 3]) -> GrowthPolicy {
        let mut g = growth();
        if let Some(weight) = weight {
            g["attested_unit_weight"] = serde_json::json!(weight);
        }
        for (index, required) in tier_attested.iter().enumerate() {
            g["tiers"][index]["attested_units"] = serde_json::json!(required);
        }
        parse_growth_policy(&policy_json(g), &["growth-test-v1"]).expect("valid")
    }

    #[test]
    fn attested_fields_are_optional_and_default_to_no_change() {
        let policy = policy();
        assert_eq!(policy.attested_unit_weight(), 1);
        assert!(policy.tiers().iter().all(|tier| tier.attested_units == 0));
        let explicit = attested_policy(Some(3), [0, 1, 2]);
        assert_eq!(explicit.attested_unit_weight(), 3);
        assert_eq!(
            explicit
                .tiers()
                .iter()
                .map(|t| t.attested_units)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn malformed_attested_fields_are_refused() {
        let v = &["growth-test-v1"];
        let with_growth = |mutate: &dyn Fn(&mut serde_json::Value)| {
            let mut g = growth();
            mutate(&mut g);
            policy_json(g)
        };
        let refused = [
            (
                "zero weight",
                with_growth(&|g| g["attested_unit_weight"] = serde_json::json!(0)),
            ),
            (
                "negative weight",
                with_growth(&|g| g["attested_unit_weight"] = serde_json::json!(-1)),
            ),
            (
                "weight times cap overflows",
                with_growth(&|g| g["attested_unit_weight"] = serde_json::json!(u32::MAX)),
            ),
            (
                "tier 0 requires attestation",
                with_growth(&|g| g["tiers"][0]["attested_units"] = serde_json::json!(1)),
            ),
            (
                "non-monotone attested requirement",
                with_growth(&|g| {
                    g["tiers"][1]["attested_units"] = serde_json::json!(2);
                    g["tiers"][2]["attested_units"] = serde_json::json!(1);
                }),
            ),
            (
                "attested requirement above the tier's units",
                with_growth(&|g| g["tiers"][1]["attested_units"] = serde_json::json!(4)),
            ),
            (
                "unknown tier field",
                with_growth(&|g| g["tiers"][1]["attested"] = serde_json::json!(1)),
            ),
        ];
        for (label, json) in refused {
            assert!(parse_growth_policy(&json, v).is_err(), "{label}");
        }
    }

    #[test]
    fn an_attested_acceptance_qualifies_without_the_gate_or_quality_test() {
        // (b): no gate fact at all, yet the unit counts.
        let mut f = Facts::new();
        f.attested(1);
        let e = evaluate(&policy(), &f.0, t(2));
        assert_eq!((e.units, e.attested_units, e.active_weeks), (1, 1, 1));
        // A gate-passed and attested submission is still one unit.
        let mut both = Facts::new();
        let s = both.qualified(1);
        both.push("attested_submission", s, "attested_provider_tee", t(1));
        let e = evaluate(&policy(), &both.0, t(2));
        assert_eq!((e.units, e.attested_units), (1, 1));
        // A failed gate does not stop an attested unit: attestation stands in
        // for the test, it is not an extra condition on it.
        let mut failed = Facts::new();
        let s = failed.attested(1);
        failed.push("gate_evaluation", s, "evaluated_failed", t(1));
        assert_eq!(evaluate(&policy(), &failed.0, t(2)).units, 1);
    }

    #[test]
    fn each_requirement_of_an_attested_unit_is_checked() {
        let base = |mutate: &dyn Fn(&mut Facts)| {
            let mut f = Facts::new();
            f.attested(1);
            mutate(&mut f);
            let e = evaluate(&policy(), &f.0, t(2));
            (e.units, e.attested_units)
        };
        assert_eq!(base(&|_| {}), (1, 1));
        assert_eq!(
            base(&|f| f.0.retain(|x| x.source_kind != "accepted_submission")),
            (0, 0),
            "attestation alone is not acceptance"
        );
        assert_eq!(
            base(&|f| f.0[0].outcome = "accepted_self_reviewed".into()),
            (0, 0)
        );
        assert_eq!(
            base(&|f| f.last_attested().first_in_cluster = Some(false)),
            (0, 0),
            "a later member of a dedup cluster earns nothing, attested or not"
        );
        assert_eq!(base(&|f| f.last_attested().first_in_cluster = None), (0, 0));
        assert_eq!(
            base(&|f| f.last_attested().outcome = "attested_gateway".into()),
            (0, 0),
            "only the provider-TEE outcome counts"
        );
        assert_eq!(
            base(&|f| f.last_attested().source_kind = "attested_other".into()),
            (0, 0)
        );
    }

    #[test]
    fn netting_and_penalties_remove_attested_units_too() {
        for (kind, outcome) in [
            ("submission_withdrawn", "withdrawn"),
            ("submission_revoked", "revoked"),
            ("submission_quarantined", "quarantined"),
        ] {
            let mut f = Facts::new();
            let gone = f.attested(1);
            f.push(kind, gone, outcome, t(2));
            let e = evaluate(&policy(), &f.0, t(3));
            assert_eq!((e.units, e.attested_units), (0, 0), "{kind}");
        }
        let mut f = Facts::new();
        let s = f.attested(1);
        f.push("abuse_penalty", s, "penalized", t(2));
        let e = evaluate(&policy(), &f.0, t(30));
        assert_eq!(
            (e.units, e.attested_units),
            (0, 0),
            "an attested unit accepted before a penalty never counts again"
        );
    }

    #[test]
    fn the_attested_weight_buys_units_but_the_weekly_cap_still_binds() {
        // (a): weight 2, weekly cap 3.
        let policy = attested_policy(Some(2), [0, 0, 0]);
        let mut f = Facts::new();
        f.attested(1);
        let e = evaluate(&policy, &f.0, t(2));
        assert_eq!((e.units, e.attested_units, e.units_capped), (2, 1, 0));
        // One attested (2) and one plain (1) fill the week exactly.
        f.qualified(1);
        let e = evaluate(&policy, &f.0, t(2));
        assert_eq!((e.units, e.attested_units, e.units_capped), (3, 1, 0));
        // Five attested in one week: 10 weighted units, capped at 3; the
        // attested count is capped at the same 3.
        let mut flood = Facts::new();
        for _ in 0..5 {
            flood.attested(1);
        }
        let e = evaluate(&policy, &flood.0, t(2));
        assert_eq!((e.units, e.attested_units, e.units_capped), (3, 3, 7));
        assert_eq!(e.active_weeks, 1);
    }

    #[test]
    fn a_tier_can_require_attested_units() {
        // (c): tier 1 needs one attested unit, tier 2 needs two.
        let policy = attested_policy(None, [0, 1, 2]);
        let mut plain = Facts::new();
        for week in 0..3 {
            for _ in 0..3 {
                plain.qualified(week * 7 + 1);
            }
        }
        let e = evaluate(&policy, &plain.0, t(22));
        assert_eq!((e.units, e.attested_units), (9, 0));
        assert_eq!(e.tier, 0, "units alone no longer reach tier 1");
        let mut mixed = plain;
        mixed.attested(15);
        let e = evaluate(&policy, &mixed.0, t(22));
        assert_eq!(e.attested_units, 1);
        assert_eq!(e.tier, 1);
        mixed.attested(16);
        let e = evaluate(&policy, &mixed.0, t(22));
        assert_eq!(e.tier, 2);
    }

    #[test]
    fn attested_facts_enter_the_digest_and_reproduce() {
        let mut f = Facts::new();
        f.qualified(1);
        let plain = evaluate(&policy(), &f.0, t(2));
        f.attested(1);
        let with_attested = evaluate(&policy(), &f.0, t(2));
        assert_ne!(plain.facts_digest, with_attested.facts_digest);
        let mut reversed = f.0.clone();
        reversed.reverse();
        assert_eq!(with_attested, evaluate(&policy(), &reversed, t(2)));
        let mut moved = f.0.clone();
        moved
            .iter_mut()
            .rev()
            .find(|x| x.source_kind == "attested_submission")
            .unwrap()
            .first_in_cluster = Some(false);
        assert_ne!(
            with_attested.facts_digest,
            evaluate(&policy(), &moved, t(2)).facts_digest
        );
    }

    #[test]
    fn the_evaluation_serializes_without_identifiers() {
        let mut f = Facts::new();
        let s = f.qualified(1);
        let rendered = serde_json::to_string(&evaluate(&policy(), &f.0, t(2))).unwrap();
        assert!(!rendered.contains(&s.to_string()));
        for fact in &f.0 {
            assert!(!rendered.contains(&fact.source_id.to_string()));
        }
        let _: BTreeMap<String, serde_json::Value> = serde_json::from_str(&rendered).unwrap();
        let _ = WEEK_SECONDS;
    }
}
