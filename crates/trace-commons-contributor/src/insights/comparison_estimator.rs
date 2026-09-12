//! Pure deterministic task-level outcome resampling.
//!
//! This does not make a calibrated comparison rule available to saved
//! specifications. It provides the bounded implementation and calibration
//! harness needed to choose and freeze such a rule without changing the
//! existing saved-specification ABI prematurely.

use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const BASIS_POINTS: i128 = 10_000;
const MAX_TASKS: usize = 256;
const MIN_REPLICATES: u32 = 1_000;
const MAX_REPLICATES: u32 = 100_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AssessedOutcome {
    Accepted,
    Partial,
    Rejected,
}

impl AssessedOutcome {
    pub const ALL: [Self; 3] = [Self::Accepted, Self::Partial, Self::Rejected];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EstimatorTaskV1 {
    pub task_id: String,
    pub cohort_label: String,
    pub outcome: AssessedOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EstimatorInputV1 {
    pub specification_digest: String,
    /// Canonical label order fixes each contrast as index 1 minus index 0.
    pub cohort_labels: [String; 2],
    pub tasks: Vec<EstimatorTaskV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EstimatorRulesV1 {
    pub replicates: u32,
    pub familywise_confidence_bps: u16,
    pub maximum_interval_width_bps: u16,
    /// V1 admits one exact stratum, hence a fixed weight of one.
    pub exact_stratum_weight_bps: u16,
    pub interval_method: IntervalMethod,
    pub multiplicity_policy: MultiplicityPolicy,
    pub boundary_policy: BoundaryPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntervalMethod {
    PercentileBootstrap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MultiplicityPolicy {
    BonferroniThreeOutcomeContrasts,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    SuppressHomogeneousCells,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EstimateState {
    ObservedDifference,
    UncertainDifference,
    InsufficientPrecision,
    IndeterminateBoundary,
    InsufficientResamplingSupport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutcomeContrastV1 {
    pub outcome: AssessedOutcome,
    /// Rounded presentation values; decisions use the exact ratios below.
    pub first_cohort_proportion_bps: Option<u16>,
    pub second_cohort_proportion_bps: Option<u16>,
    pub observed_difference_bps: Option<i32>,
    pub exact_difference_numerator: Option<i64>,
    pub exact_difference_denominator: Option<u64>,
    pub interval_lower_numerator: Option<i64>,
    pub interval_upper_numerator: Option<i64>,
    pub interval_denominator: Option<u64>,
    pub state: EstimateState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskOutcomeEstimateV1 {
    pub schema_version: u32,
    pub seed_digest: String,
    pub assessed_estimation_input_digest: String,
    pub first_cohort_assessed_tasks: u64,
    pub second_cohort_assessed_tasks: u64,
    pub contrasts: Vec<OutcomeContrastV1>,
}

#[derive(Serialize)]
struct SeedFields<'a> {
    specification_digest: &'a str,
    assessed_estimation_input_digest: &'a str,
    cohort_labels: &'a [String; 2],
    outcomes: &'a [AssessedOutcome; 3],
    rules: &'a EstimatorRulesV1,
    tasks: &'a [EstimatorTaskV1],
}

pub fn estimate_task_outcomes(
    input: &EstimatorInputV1,
    rules: &EstimatorRulesV1,
) -> Result<TaskOutcomeEstimateV1> {
    validate(input, rules)?;
    let mut tasks = input.tasks.clone();
    tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    let first = cell(&tasks, &input.cohort_labels[0]);
    let second = cell(&tasks, &input.cohort_labels[1]);
    let denominator = common_denominator(first.len(), second.len())?;
    let assessed_estimation_input_digest = assessed_input_digest(&tasks)?;
    let seed_digest = make_seed_digest(input, rules, &tasks, &assessed_estimation_input_digest)?;
    let seed = decode_digest(&seed_digest)?;
    let supported = first.len() >= 2 && second.len() >= 2;
    let mut samples = AssessedOutcome::ALL.map(|_| Vec::new());
    if supported {
        for values in &mut samples {
            values.reserve(rules.replicates as usize);
        }
        for replicate in 0..rules.replicates {
            let first_counts = resample_counts(&seed, replicate, 0, &first);
            let second_counts = resample_counts(&seed, replicate, 1, &second);
            for index in 0..3 {
                samples[index].push(difference_numerator(
                    first_counts[index],
                    first.len(),
                    second_counts[index],
                    second.len(),
                )?);
            }
        }
        for values in &mut samples {
            values.sort_unstable();
        }
    }

    let mut contrasts = Vec::with_capacity(3);
    for (index, outcome) in AssessedOutcome::ALL.into_iter().enumerate() {
        let first_count = first.iter().filter(|task| task.outcome == outcome).count();
        let second_count = second.iter().filter(|task| task.outcome == outcome).count();
        let estimand_defined = !first.is_empty() && !second.is_empty();
        let observed = estimand_defined
            .then(|| difference_numerator(first_count, first.len(), second_count, second.len()))
            .transpose()?;
        let homogeneous = first_count == 0
            || first_count == first.len()
            || second_count == 0
            || second_count == second.len();
        let (lower, upper, interval_denominator, state) = if !supported {
            (
                None,
                None,
                None,
                EstimateState::InsufficientResamplingSupport,
            )
        } else {
            let (lower, upper) = simultaneous_interval(&samples[index], rules)?;
            let width_scaled = i128::from(upper - lower)
                .checked_mul(BASIS_POINTS)
                .ok_or_else(invalid)?;
            let threshold_scaled = i128::from(rules.maximum_interval_width_bps)
                .checked_mul(i128::from(denominator))
                .ok_or_else(invalid)?;
            let state =
                if homogeneous || lower == 0 || upper == 0 || width_scaled == threshold_scaled {
                    EstimateState::IndeterminateBoundary
                } else if width_scaled > threshold_scaled {
                    EstimateState::InsufficientPrecision
                } else if lower < 0 && upper > 0 {
                    EstimateState::UncertainDifference
                } else {
                    EstimateState::ObservedDifference
                };
            (Some(lower), Some(upper), Some(denominator), state)
        };
        contrasts.push(OutcomeContrastV1 {
            outcome,
            first_cohort_proportion_bps: (!first.is_empty())
                .then(|| display_proportion(first_count, first.len()))
                .transpose()?,
            second_cohort_proportion_bps: (!second.is_empty())
                .then(|| display_proportion(second_count, second.len()))
                .transpose()?,
            observed_difference_bps: observed
                .map(|value| display_ratio(value, denominator))
                .transpose()?,
            exact_difference_numerator: observed,
            exact_difference_denominator: estimand_defined.then_some(denominator),
            interval_lower_numerator: lower,
            interval_upper_numerator: upper,
            interval_denominator,
            state,
        });
    }
    Ok(TaskOutcomeEstimateV1 {
        schema_version: 1,
        seed_digest,
        assessed_estimation_input_digest,
        first_cohort_assessed_tasks: first.len() as u64,
        second_cohort_assessed_tasks: second.len() as u64,
        contrasts,
    })
}

fn validate(input: &EstimatorInputV1, rules: &EstimatorRulesV1) -> Result<()> {
    if !valid_digest(&input.specification_digest)
        || input.cohort_labels[0] >= input.cohort_labels[1]
        || input.cohort_labels.iter().any(|label| !valid_label(label))
        || input.tasks.len() > MAX_TASKS
        || !(MIN_REPLICATES..=MAX_REPLICATES).contains(&rules.replicates)
        || !(8_000..10_000).contains(&rules.familywise_confidence_bps)
        || rules.maximum_interval_width_bps == 0
        || u32::from(rules.maximum_interval_width_bps) >= 20_000
        || rules.exact_stratum_weight_bps != 10_000
    {
        return Err(invalid());
    }
    let selected = input.cohort_labels.iter().collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    for task in &input.tasks {
        if uuid::Uuid::parse_str(&task.task_id)
            .ok()
            .is_none_or(|id| id.is_nil() || id.to_string() != task.task_id)
            || !selected.contains(&task.cohort_label)
            || !ids.insert(&task.task_id)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn cell<'a>(tasks: &'a [EstimatorTaskV1], cohort: &str) -> Vec<&'a EstimatorTaskV1> {
    tasks
        .iter()
        .filter(|task| task.cohort_label == cohort)
        .collect()
}

fn make_seed_digest(
    input: &EstimatorInputV1,
    rules: &EstimatorRulesV1,
    ordered_tasks: &[EstimatorTaskV1],
    assessed_estimation_input_digest: &str,
) -> Result<String> {
    let bytes = serde_json::to_vec(&SeedFields {
        specification_digest: &input.specification_digest,
        assessed_estimation_input_digest,
        cohort_labels: &input.cohort_labels,
        outcomes: &AssessedOutcome::ALL,
        rules,
        tasks: ordered_tasks,
    })?;
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-task-outcome-estimator-seed-v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

fn assessed_input_digest(ordered_tasks: &[EstimatorTaskV1]) -> Result<String> {
    let bytes = serde_json::to_vec(ordered_tasks)?;
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-assessed-outcome-estimation-input-v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

fn resample_counts(
    seed: &[u8; 32],
    replicate: u32,
    cell_index: u8,
    tasks: &[&EstimatorTaskV1],
) -> [usize; 3] {
    let mut counts = [0usize; 3];
    for draw in 0..tasks.len() {
        let selected = unbiased_index(seed, replicate, cell_index, draw as u32, tasks.len());
        counts[outcome_index(tasks[selected].outcome)] += 1;
    }
    counts
}

fn unbiased_index(seed: &[u8; 32], replicate: u32, cell: u8, draw: u32, len: usize) -> usize {
    let modulus = len as u128;
    let limit = ((u64::MAX as u128 + 1) / modulus) * modulus;
    for retry in 0u32.. {
        let mut hash = Sha256::new();
        hash.update(b"trace-commons-task-outcome-estimator-draw-v1\0");
        hash.update(seed);
        hash.update(replicate.to_be_bytes());
        hash.update([cell]);
        hash.update(draw.to_be_bytes());
        hash.update(retry.to_be_bytes());
        let bytes = hash.finalize();
        let value = u64::from_be_bytes(bytes[..8].try_into().expect("eight bytes")) as u128;
        if value < limit {
            return (value % modulus) as usize;
        }
    }
    unreachable!("nonzero finite modulus admits a hash value")
}

fn simultaneous_interval(samples: &[i64], rules: &EstimatorRulesV1) -> Result<(i64, i64)> {
    let alpha = 10_000u64 - u64::from(rules.familywise_confidence_bps);
    let quantile_denominator = 2 * 3 * 10_000u64;
    let count = samples.len() as u64;
    let lower = count.checked_mul(alpha).ok_or_else(invalid)? / quantile_denominator;
    let upper_numerator = count
        .checked_mul(quantile_denominator - alpha)
        .ok_or_else(invalid)?;
    let upper = upper_numerator
        .div_ceil(quantile_denominator)
        .saturating_sub(1)
        .min(count - 1);
    Ok((samples[lower as usize], samples[upper as usize]))
}

fn common_denominator(first: usize, second: usize) -> Result<u64> {
    if first == 0 || second == 0 {
        return Ok(1);
    }
    (first as u64)
        .checked_mul(second as u64)
        .ok_or_else(invalid)
}

fn difference_numerator(
    first_count: usize,
    first_total: usize,
    second_count: usize,
    second_total: usize,
) -> Result<i64> {
    if first_total == 0 || second_total == 0 {
        return Ok(0);
    }
    let second = (second_count as i128)
        .checked_mul(first_total as i128)
        .ok_or_else(invalid)?;
    let first = (first_count as i128)
        .checked_mul(second_total as i128)
        .ok_or_else(invalid)?;
    i64::try_from(second - first).map_err(|_| invalid())
}

fn display_proportion(count: usize, total: usize) -> Result<u16> {
    if total == 0 {
        return Ok(0);
    }
    let numerator = (count as u64)
        .checked_mul(10_000)
        .and_then(|value| value.checked_add(total as u64 / 2))
        .ok_or_else(invalid)?;
    u16::try_from(numerator / total as u64).map_err(|_| invalid())
}

fn display_ratio(numerator: i64, denominator: u64) -> Result<i32> {
    let scaled = i128::from(numerator)
        .checked_mul(BASIS_POINTS)
        .ok_or_else(invalid)?;
    let half = i128::from(denominator / 2);
    let rounded = if scaled >= 0 {
        scaled + half
    } else {
        scaled - half
    } / i128::from(denominator);
    i32::try_from(rounded).map_err(|_| invalid())
}

fn outcome_index(outcome: AssessedOutcome) -> usize {
    match outcome {
        AssessedOutcome::Accepted => 0,
        AssessedOutcome::Partial => 1,
        AssessedOutcome::Rejected => 2,
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/+-".contains(&byte))
}

fn decode_digest(value: &str) -> Result<[u8; 32]> {
    let mut decoded = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(decoded)
}

fn invalid() -> anyhow::Error {
    anyhow::anyhow!("insights-comparison-estimator-invalid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> EstimatorRulesV1 {
        EstimatorRulesV1 {
            replicates: 2_000,
            familywise_confidence_bps: 9_500,
            maximum_interval_width_bps: 8_000,
            exact_stratum_weight_bps: 10_000,
            interval_method: IntervalMethod::PercentileBootstrap,
            multiplicity_policy: MultiplicityPolicy::BonferroniThreeOutcomeContrasts,
            boundary_policy: BoundaryPolicy::SuppressHomogeneousCells,
        }
    }

    fn task(index: u64, cohort: &str, outcome: AssessedOutcome) -> EstimatorTaskV1 {
        EstimatorTaskV1 {
            task_id: format!("00000000-0000-4000-8000-{index:012}"),
            cohort_label: cohort.into(),
            outcome,
        }
    }

    fn input(first: &[AssessedOutcome], second: &[AssessedOutcome]) -> EstimatorInputV1 {
        let mut tasks = Vec::new();
        for (index, outcome) in first.iter().enumerate() {
            tasks.push(task(index as u64 + 1, "model-a", *outcome));
        }
        for (index, outcome) in second.iter().enumerate() {
            tasks.push(task(index as u64 + 129, "model-b", *outcome));
        }
        EstimatorInputV1 {
            specification_digest: "11".repeat(32),
            cohort_labels: ["model-a".into(), "model-b".into()],
            tasks,
        }
    }

    fn outcomes(accepted: usize, partial: usize, rejected: usize) -> Vec<AssessedOutcome> {
        std::iter::repeat_n(AssessedOutcome::Accepted, accepted)
            .chain(std::iter::repeat_n(AssessedOutcome::Partial, partial))
            .chain(std::iter::repeat_n(AssessedOutcome::Rejected, rejected))
            .collect()
    }

    #[test]
    fn known_distribution_preserves_exact_task_level_estimands() {
        let estimate = estimate_task_outcomes(
            &input(&outcomes(20, 10, 10), &outcomes(10, 10, 20)),
            &rules(),
        )
        .unwrap();
        assert_eq!(estimate.first_cohort_assessed_tasks, 40);
        assert_eq!(estimate.second_cohort_assessed_tasks, 40);
        assert_eq!(estimate.contrasts[0].exact_difference_numerator, Some(-400));
        assert_eq!(
            estimate.contrasts[0].exact_difference_denominator,
            Some(1_600)
        );
        assert_eq!(estimate.contrasts[0].observed_difference_bps, Some(-2_500));
        assert_eq!(estimate.contrasts[1].observed_difference_bps, Some(0));
        assert_eq!(estimate.contrasts[2].observed_difference_bps, Some(2_500));
        assert_eq!(
            estimate.contrasts[1].state,
            EstimateState::UncertainDifference
        );
    }

    #[test]
    fn exact_no_difference_is_uncertain_not_a_tie() {
        let values = outcomes(12, 10, 8);
        let estimate = estimate_task_outcomes(&input(&values, &values), &rules()).unwrap();
        assert!(
            estimate
                .contrasts
                .iter()
                .all(|contrast| contrast.exact_difference_numerator == Some(0))
        );
        assert!(estimate.contrasts.iter().all(|contrast| {
            matches!(
                contrast.state,
                EstimateState::UncertainDifference | EstimateState::InsufficientPrecision
            )
        }));
    }

    #[test]
    fn sparse_cell_has_no_interval_or_difference_claim() {
        let estimate = estimate_task_outcomes(
            &input(
                &[AssessedOutcome::Accepted],
                &[AssessedOutcome::Accepted, AssessedOutcome::Rejected],
            ),
            &rules(),
        )
        .unwrap();
        assert!(estimate.contrasts.iter().all(|contrast| {
            contrast.state == EstimateState::InsufficientResamplingSupport
                && contrast.interval_lower_numerator.is_none()
                && contrast.interval_upper_numerator.is_none()
        }));
    }

    #[test]
    fn empty_cell_has_typed_unavailable_estimand() {
        let estimate = estimate_task_outcomes(
            &input(&[], &[AssessedOutcome::Accepted, AssessedOutcome::Rejected]),
            &rules(),
        )
        .unwrap();
        assert!(estimate.contrasts.iter().all(|contrast| {
            contrast.first_cohort_proportion_bps.is_none()
                && contrast.observed_difference_bps.is_none()
                && contrast.exact_difference_numerator.is_none()
                && contrast.exact_difference_denominator.is_none()
                && contrast.state == EstimateState::InsufficientResamplingSupport
        }));
    }

    #[test]
    fn homogeneous_cells_are_boundary_suppressed() {
        let estimate =
            estimate_task_outcomes(&input(&outcomes(20, 0, 0), &outcomes(0, 0, 20)), &rules())
                .unwrap();
        assert!(
            estimate
                .contrasts
                .iter()
                .all(|contrast| contrast.state == EstimateState::IndeterminateBoundary)
        );
    }

    #[test]
    fn imbalanced_cells_remain_task_weighted_and_visible_as_imprecise() {
        let mut narrow = rules();
        narrow.maximum_interval_width_bps = 1_000;
        let estimate =
            estimate_task_outcomes(&input(&outcomes(2, 1, 1), &outcomes(20, 10, 10)), &narrow)
                .unwrap();
        assert_eq!(estimate.first_cohort_assessed_tasks, 4);
        assert_eq!(estimate.second_cohort_assessed_tasks, 40);
        assert!(estimate.contrasts.iter().all(|contrast| matches!(
            contrast.state,
            EstimateState::InsufficientPrecision | EstimateState::IndeterminateBoundary
        )));
    }

    #[test]
    fn task_ids_are_unique_and_attempt_duplication_has_no_input_shape() {
        let mut value = input(&outcomes(2, 1, 1), &outcomes(2, 1, 1));
        value.tasks.push(value.tasks[0].clone());
        assert!(estimate_task_outcomes(&value, &rules()).is_err());
    }

    #[test]
    fn import_order_does_not_change_seed_or_result() {
        let first = input(&outcomes(12, 10, 8), &outcomes(8, 10, 12));
        let mut reversed = first.clone();
        reversed.tasks.reverse();
        let first = estimate_task_outcomes(&first, &rules()).unwrap();
        let second = estimate_task_outcomes(&reversed, &rules()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.seed_digest,
            "2045586eb1f0510bb7a9e309bf2a1d24b3cc345075b93ad047890b86329ce263"
        );
    }

    #[test]
    fn rule_bytes_and_consumed_digest_change_seed() {
        let value = input(&outcomes(4, 3, 3), &outcomes(3, 3, 4));
        let baseline = estimate_task_outcomes(&value, &rules()).unwrap();
        let mut changed_rules = rules();
        changed_rules.replicates += 1;
        assert_ne!(
            baseline.seed_digest,
            estimate_task_outcomes(&value, &changed_rules)
                .unwrap()
                .seed_digest
        );
        let mut changed_input = value;
        changed_input.tasks[0].outcome = AssessedOutcome::Rejected;
        assert_ne!(
            baseline.seed_digest,
            estimate_task_outcomes(&changed_input, &rules())
                .unwrap()
                .seed_digest
        );
    }

    #[test]
    fn equality_at_precision_threshold_is_indeterminate() {
        let value = input(&outcomes(4, 3, 3), &outcomes(3, 3, 4));
        let broad = estimate_task_outcomes(&value, &rules()).unwrap();
        let contrast = &broad.contrasts[0];
        let width =
            contrast.interval_upper_numerator.unwrap() - contrast.interval_lower_numerator.unwrap();
        let scaled_numerator = i128::from(width) * BASIS_POINTS;
        let denominator = i128::from(contrast.interval_denominator.unwrap());
        assert_eq!(scaled_numerator % denominator, 0);
        let scaled = scaled_numerator / denominator;
        let mut exact = rules();
        exact.maximum_interval_width_bps = u16::try_from(scaled).unwrap();
        let estimate = estimate_task_outcomes(&value, &exact).unwrap();
        assert_eq!(
            estimate.contrasts[0].state,
            EstimateState::IndeterminateBoundary
        );
    }

    #[test]
    fn calibration_grid_never_promotes_homogeneous_or_sparse_nulls() {
        for size in [1usize, 2, 5, 20, 80] {
            for accepted in [0usize, size / 2, size] {
                let partial = (size - accepted) / 2;
                let rejected = size - accepted - partial;
                let values = outcomes(accepted, partial, rejected);
                let estimate = estimate_task_outcomes(&input(&values, &values), &rules()).unwrap();
                assert!(
                    estimate
                        .contrasts
                        .iter()
                        .all(|contrast| contrast.state != EstimateState::ObservedDifference)
                );
            }
        }
    }

    fn sampled_outcomes(
        size: usize,
        probabilities: [u16; 3],
        experiment: u32,
        cohort: u8,
    ) -> Vec<AssessedOutcome> {
        assert_eq!(
            probabilities
                .iter()
                .map(|value| u32::from(*value))
                .sum::<u32>(),
            10_000
        );
        (0..size)
            .map(|draw| {
                let mut hash = Sha256::new();
                hash.update(b"trace-commons-estimator-calibration-fixture-v1\0");
                hash.update(experiment.to_be_bytes());
                hash.update([cohort]);
                hash.update((draw as u32).to_be_bytes());
                let bytes = hash.finalize();
                let bucket =
                    u32::from(u16::from_be_bytes(bytes[..2].try_into().unwrap())) * 10_000 / 65_536;
                if bucket < u32::from(probabilities[0]) {
                    AssessedOutcome::Accepted
                } else if bucket < u32::from(probabilities[0]) + u32::from(probabilities[1]) {
                    AssessedOutcome::Partial
                } else {
                    AssessedOutcome::Rejected
                }
            })
            .collect()
    }

    #[test]
    fn repeated_calibration_grid_measures_null_error_and_interval_coverage() {
        let mut calibration_rules = rules();
        calibration_rules.replicates = 1_000;
        calibration_rules.maximum_interval_width_bps = 19_999;
        let distributions = [
            [500, 1_500, 8_000],
            [3_300, 3_400, 3_300],
            [8_000, 1_500, 500],
        ];
        let sizes = [(8usize, 32usize), (20, 20), (32, 8)];
        let mut false_positive_experiments = 0u32;
        let mut null_covered = 0u32;
        let mut null_total = 0u32;
        for (scenario, probabilities) in distributions.into_iter().enumerate() {
            for experiment in 0..8u32 {
                let first = sampled_outcomes(sizes[scenario].0, probabilities, experiment, 0);
                let second = sampled_outcomes(sizes[scenario].1, probabilities, experiment, 1);
                let estimate =
                    estimate_task_outcomes(&input(&first, &second), &calibration_rules).unwrap();
                false_positive_experiments += u32::from(
                    estimate
                        .contrasts
                        .iter()
                        .any(|value| value.state == EstimateState::ObservedDifference),
                );
                for value in &estimate.contrasts {
                    if let (Some(lower), Some(upper)) = (
                        value.interval_lower_numerator,
                        value.interval_upper_numerator,
                    ) {
                        null_total += 1;
                        null_covered += u32::from(lower <= 0 && upper >= 0);
                    }
                }
            }
        }
        assert!(
            false_positive_experiments <= 3,
            "false_positive_experiments={false_positive_experiments}"
        );
        assert!(
            null_covered * 100 < null_total * 90,
            "this candidate must remain unavailable unless the known sparse-grid undercoverage is resolved: covered={null_covered}, total={null_total}"
        );

        let mut alternative_covered = 0u32;
        for experiment in 0..12u32 {
            let first = sampled_outcomes(30, [3_000, 3_000, 4_000], experiment, 2);
            let second = sampled_outcomes(30, [5_000, 2_000, 3_000], experiment, 3);
            let estimate =
                estimate_task_outcomes(&input(&first, &second), &calibration_rules).unwrap();
            let accepted = &estimate.contrasts[0];
            let denominator = i128::from(accepted.interval_denominator.unwrap());
            let truth = denominator * 2_000 / 10_000;
            alternative_covered += u32::from(
                i128::from(accepted.interval_lower_numerator.unwrap()) <= truth
                    && truth <= i128::from(accepted.interval_upper_numerator.unwrap()),
            );
        }
        assert_eq!((null_covered, null_total), (60, 72));
        assert_eq!(false_positive_experiments, 0);
        assert_eq!(alternative_covered, 12);
        assert!(
            alternative_covered >= 10,
            "alternative_covered={alternative_covered}"
        );

        let artifact: serde_json::Value = serde_json::from_str(include_str!(
            "../../fixtures/insights/comparison-estimator/percentile-bootstrap-candidate-v1.json"
        ))
        .unwrap();
        assert_eq!(artifact["qualified_for_saved_specifications"], false);
        assert_eq!(artifact["null_contrasts_covered"], null_covered);
        assert_eq!(artifact["null_contrasts_total"], null_total);
        assert_eq!(
            artifact["null_experiments_with_observed_difference"],
            false_positive_experiments
        );
        assert_eq!(
            artifact["alternative_accepted_contrasts_covered"],
            alternative_covered
        );
    }
}
