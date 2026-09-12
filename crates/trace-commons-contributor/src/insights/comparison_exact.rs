use anyhow::Result;
use serde::{Deserialize, Serialize};

const MAX_CALIBRATION_EXPERIMENTS: usize = 10_000;
const EXACT_GRID: u32 = 1_000_000;
const BONFERRONI_ONE_SIDED_DENOMINATOR: u32 = 240;
pub(crate) const MAX_TASKS: usize = 256;
pub const EXACT_CANDIDATE_METHOD: &str = "exact_binomial_components_bonferroni_v1";
pub const EXACT_CANDIDATE_PROTOCOL_SHA256: &str =
    "d257605993aaa94220ce31144203c0c05a28ff6c33cc129d678f90a8986e26c8";
const EXACT_MAXIMUM_WIDTH_MILLIONTHS: i64 = 500_000;

/// Candidate exact component interval on a fixed millionth grid.
///
/// Each of the six cohort-by-outcome binomial components receives two tails
/// of probability 1/240. The twelve-tail union bound therefore limits family
/// noncoverage to 5%, without assuming independence between outcome counts.
/// Availability to saved specifications is controlled separately by the
/// estimator state frozen into each immutable specification.
pub(crate) fn exact_component_interval(successes: usize, total: usize) -> Result<(u32, u32)> {
    if total > MAX_TASKS {
        return Err(invalid());
    }
    exact_binomial_interval(successes, total, BONFERRONI_ONE_SIDED_DENOMINATOR)
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExactCandidateCounts {
    pub total: usize,
    pub outcomes: [usize; 3],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExactCandidateDecision {
    InsufficientPrecision,
    IndeterminateBoundary,
    ExcludesZero,
    IncludesZero,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExactCandidateComponentInterval {
    pub lower_millionths: u32,
    pub upper_millionths: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExactCandidateContrast {
    pub lower_millionths: i64,
    pub upper_millionths: i64,
    pub width_millionths: i64,
    pub decision: ExactCandidateDecision,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExactCandidateEvaluation {
    Supported {
        first_components: [ExactCandidateComponentInterval; 3],
        second_components: [ExactCandidateComponentInterval; 3],
        contrasts: [ExactCandidateContrast; 3],
    },
    SuppressedBelowMinimumCohortSupport,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExactCandidateEvaluationStatus {
    Supported,
    SuppressedBelowMinimumCohortSupport,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactCandidateEvaluationWire {
    status: ExactCandidateEvaluationStatus,
    #[serde(default, deserialize_with = "present_non_null")]
    first_components: Option<[ExactCandidateComponentInterval; 3]>,
    #[serde(default, deserialize_with = "present_non_null")]
    second_components: Option<[ExactCandidateComponentInterval; 3]>,
    #[serde(default, deserialize_with = "present_non_null")]
    contrasts: Option<[ExactCandidateContrast; 3]>,
}

fn present_non_null<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for ExactCandidateEvaluation {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ExactCandidateEvaluationWire::deserialize(deserializer)?;
        match (
            wire.status,
            wire.first_components,
            wire.second_components,
            wire.contrasts,
        ) {
            (
                ExactCandidateEvaluationStatus::Supported,
                Some(first_components),
                Some(second_components),
                Some(contrasts),
            ) => Ok(Self::Supported {
                first_components,
                second_components,
                contrasts,
            }),
            (
                ExactCandidateEvaluationStatus::SuppressedBelowMinimumCohortSupport,
                None,
                None,
                None,
            ) => Ok(Self::SuppressedBelowMinimumCohortSupport),
            _ => Err(serde::de::Error::custom(
                "invalid exact candidate evaluation fields",
            )),
        }
    }
}

pub(crate) fn combine_exact_candidate_intervals(
    first: (u32, u32),
    second: (u32, u32),
) -> ExactCandidateContrast {
    let lower = i64::from(second.0) - i64::from(first.1);
    let upper = i64::from(second.1) - i64::from(first.0);
    let width = upper - lower;
    let decision = if width > EXACT_MAXIMUM_WIDTH_MILLIONTHS {
        ExactCandidateDecision::InsufficientPrecision
    } else if width == EXACT_MAXIMUM_WIDTH_MILLIONTHS || lower == 0 || upper == 0 {
        ExactCandidateDecision::IndeterminateBoundary
    } else if upper < 0 || lower > 0 {
        ExactCandidateDecision::ExcludesZero
    } else {
        ExactCandidateDecision::IncludesZero
    };
    ExactCandidateContrast {
        lower_millionths: lower,
        upper_millionths: upper,
        width_millionths: width,
        decision,
    }
}

pub(crate) fn evaluate_exact_candidate_counts(
    first: &ExactCandidateCounts,
    second: &ExactCandidateCounts,
) -> Result<ExactCandidateEvaluation> {
    let checked_sum = |outcomes: &[usize; 3]| {
        outcomes
            .iter()
            .try_fold(0_usize, |sum, value| sum.checked_add(*value))
    };
    if checked_sum(&first.outcomes) != Some(first.total)
        || checked_sum(&second.outcomes) != Some(second.total)
        || first
            .total
            .checked_add(second.total)
            .is_none_or(|total| total > MAX_TASKS)
    {
        return Err(invalid());
    }
    if first.total < 2 || second.total < 2 {
        return Ok(ExactCandidateEvaluation::SuppressedBelowMinimumCohortSupport);
    }
    let mut first_components = Vec::with_capacity(3);
    let mut second_components = Vec::with_capacity(3);
    let mut contrasts = Vec::with_capacity(3);
    for outcome in 0..3 {
        let first_interval = exact_component_interval(first.outcomes[outcome], first.total)?;
        let second_interval = exact_component_interval(second.outcomes[outcome], second.total)?;
        first_components.push(ExactCandidateComponentInterval {
            lower_millionths: first_interval.0,
            upper_millionths: first_interval.1,
        });
        second_components.push(ExactCandidateComponentInterval {
            lower_millionths: second_interval.0,
            upper_millionths: second_interval.1,
        });
        contrasts.push(combine_exact_candidate_intervals(
            first_interval,
            second_interval,
        ));
    }
    Ok(ExactCandidateEvaluation::Supported {
        first_components: first_components.try_into().map_err(|_| invalid())?,
        second_components: second_components.try_into().map_err(|_| invalid())?,
        contrasts: contrasts.try_into().map_err(|_| invalid())?,
    })
}

pub(crate) fn exact_binomial_interval(
    successes: usize,
    total: usize,
    one_sided_tail_denominator: u32,
) -> Result<(u32, u32)> {
    if total == 0 || total > MAX_CALIBRATION_EXPERIMENTS || successes > total {
        return Err(invalid());
    }
    let lower = if successes == 0 {
        0
    } else {
        last_grid_point_with_small_tail(
            successes,
            total,
            Tail::AtLeast,
            one_sided_tail_denominator,
        )?
    };
    let upper = if successes == total {
        EXACT_GRID
    } else {
        first_grid_point_with_small_tail(
            successes,
            total,
            Tail::AtMost,
            one_sided_tail_denominator,
        )?
    };
    if lower > upper {
        return Err(invalid());
    }
    Ok((lower, upper))
}

#[derive(Clone, Copy)]
pub(crate) enum Tail {
    AtLeast,
    AtMost,
}

#[cfg(test)]
pub(crate) fn tail_is_at_most_one_over_240(
    successes: usize,
    total: usize,
    probability_millionths: u32,
    tail: Tail,
) -> Result<bool> {
    tail_is_at_most(
        successes,
        total,
        probability_millionths,
        tail,
        BONFERRONI_ONE_SIDED_DENOMINATOR,
    )
}

fn last_grid_point_with_small_tail(
    successes: usize,
    total: usize,
    tail: Tail,
    denominator: u32,
) -> Result<u32> {
    let mut low = 0u32;
    let mut high = EXACT_GRID;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if tail_is_at_most(successes, total, middle, tail, denominator)? {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Ok(low)
}

fn first_grid_point_with_small_tail(
    successes: usize,
    total: usize,
    tail: Tail,
    denominator: u32,
) -> Result<u32> {
    let mut low = 0u32;
    let mut high = EXACT_GRID;
    while low < high {
        let middle = low + (high - low) / 2;
        if tail_is_at_most(successes, total, middle, tail, denominator)? {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    Ok(low)
}

fn tail_is_at_most(
    successes: usize,
    total: usize,
    probability_millionths: u32,
    tail: Tail,
    denominator_multiplier: u32,
) -> Result<bool> {
    if successes > total
        || total > MAX_CALIBRATION_EXPERIMENTS
        || probability_millionths > EXACT_GRID
    {
        return Err(invalid());
    }
    if denominator_multiplier == 0 {
        return Err(invalid());
    }
    if probability_millionths == EXACT_GRID {
        return Ok(match tail {
            Tail::AtMost => successes < total,
            Tail::AtLeast => false,
        });
    }
    let mut denominator = BigNat::one();
    for _ in 0..total {
        denominator = denominator.mul_small(EXACT_GRID);
    }
    let (maximum, complement) = match tail {
        Tail::AtMost => (successes, false),
        Tail::AtLeast if successes == 0 => return Ok(false),
        // P[X >= x] = 1 - P[X <= x-1]. Comparing the complement
        // algebraically keeps the recurrence bounded by the observed tail.
        Tail::AtLeast => (successes - 1, true),
    };
    let success_probability = probability_millionths;
    let failure_probability = EXACT_GRID - success_probability;
    let mut term = BigNat::one();
    for _ in 0..total {
        term = term.mul_small(failure_probability);
    }
    let mut numerator = BigNat::zero();
    for count in 0..=maximum {
        numerator.add_assign(&term);
        if count != maximum {
            term = apply_binomial_term_ratio(
                term,
                [u32::try_from(total - count)?, success_probability],
                [u32::try_from(count + 1)?, failure_probability],
            )?;
        }
    }
    if complement {
        Ok(denominator
            .mul_small(denominator_multiplier - 1)
            .cmp(&numerator.mul_small(denominator_multiplier))
            .is_le())
    } else {
        Ok(numerator
            .mul_small(denominator_multiplier)
            .cmp(&denominator)
            .is_le())
    }
}

fn apply_binomial_term_ratio(
    mut value: BigNat,
    mut numerators: [u32; 2],
    mut denominators: [u32; 2],
) -> Result<BigNat> {
    for numerator in &mut numerators {
        for denominator in &mut denominators {
            let common = gcd(*numerator, *denominator);
            *numerator /= common;
            *denominator /= common;
        }
    }
    for numerator in numerators {
        value = value.mul_small(numerator);
    }
    for denominator in denominators {
        value = value.div_exact_small(denominator)?;
    }
    Ok(value)
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

/// Minimal unsigned integer needed by exact binomial-tail comparisons.
/// Limbs are normalized little-endian base 2^32; operations never truncate.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BigNat(Vec<u32>);

impl BigNat {
    fn zero() -> Self {
        Self(Vec::new())
    }

    fn one() -> Self {
        Self(vec![1])
    }

    fn mul_small(&self, factor: u32) -> Self {
        if factor == 0 || self.0.is_empty() {
            return Self::zero();
        }
        let mut result = Vec::with_capacity(self.0.len() + 1);
        let mut carry = 0u64;
        for limb in &self.0 {
            let value = u64::from(*limb) * u64::from(factor) + carry;
            result.push(value as u32);
            carry = value >> 32;
        }
        if carry != 0 {
            result.push(carry as u32);
        }
        Self(result)
    }

    fn add_assign(&mut self, other: &Self) {
        let length = self.0.len().max(other.0.len());
        self.0.resize(length, 0);
        let mut carry = 0u64;
        for index in 0..length {
            let value = u64::from(self.0[index])
                + u64::from(other.0.get(index).copied().unwrap_or(0))
                + carry;
            self.0[index] = value as u32;
            carry = value >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
        self.normalize();
    }

    fn div_exact_small(&self, divisor: u32) -> Result<Self> {
        if divisor == 0 {
            return Err(invalid());
        }
        let mut result = vec![0u32; self.0.len()];
        let mut remainder = 0u64;
        for index in (0..self.0.len()).rev() {
            let value = (remainder << 32) | u64::from(self.0[index]);
            result[index] = (value / u64::from(divisor)) as u32;
            remainder = value % u64::from(divisor);
        }
        if remainder != 0 {
            return Err(invalid());
        }
        let mut result = Self(result);
        result.normalize();
        Ok(result)
    }

    fn normalize(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }
}

impl Ord for BigNat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }
}

impl PartialOrd for BigNat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn invalid() -> anyhow::Error {
    anyhow::anyhow!("insights-comparison-estimator-invalid")
}
