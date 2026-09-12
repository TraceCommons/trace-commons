use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{MAX_TASKS, exact_binomial_interval, exact_component_interval};

const EXPERIMENTS: u32 = 10_000;
const MAXIMUM_WIDTH: i64 = 500_000;

#[derive(Serialize)]
struct CalibrationArtifact {
    schema_version: u32,
    candidate: &'static str,
    qualified_for_saved_specifications: bool,
    frozen_protocol_fixture_sha256: String,
    experiments_per_setting: u32,
    elapsed_millis_observed: u128,
    settings: Vec<SettingResult>,
    admission: AdmissionResult,
}

#[derive(Serialize)]
struct SettingResult {
    family: &'static str,
    probability_index: u32,
    first_probability_bps: [u16; 3],
    second_probability_bps: [u16; 3],
    size_index: u32,
    first_size: usize,
    second_size: usize,
    removal_index: u32,
    assessed_removal_bps: u16,
    family_noncoverage_count: u32,
    family_noncoverage_interval_millionths: [u32; 2],
    experiments_with_any_zero_exclusion: u32,
    below_minimum_support_contrasts: u32,
    insufficient_precision_contrasts: u32,
    indeterminate_boundary_contrasts: u32,
    interval_width_millionths: [WidthSummary; 3],
}

#[derive(Clone, Copy, Serialize)]
struct WidthSummary {
    observed: u32,
    p50: Option<i64>,
    p90: Option<i64>,
    p95: Option<i64>,
    maximum: Option<i64>,
}

#[derive(Serialize)]
struct AdmissionResult {
    all_frozen_settings_completed: bool,
    settings_total: u32,
    settings_passing_noncoverage_bound: u32,
    admitted: bool,
    reason: &'static str,
}

#[derive(Default)]
struct Counts {
    total: usize,
    outcomes: [usize; 3],
}

#[test]
#[ignore = "writes the frozen 2.52 million-experiment qualification artifact"]
fn write_exact_component_full_grid_artifact() {
    let output = std::env::var_os("TRACE_COMMONS_COMPARISON_CALIBRATION_OUTPUT")
        .expect("TRACE_COMMONS_COMPARISON_CALIBRATION_OUTPUT is required");
    let protocol_bytes = include_bytes!(
        "../../fixtures/insights/comparison-estimator/exact-component-candidate-v1.json"
    );
    let protocol: serde_json::Value = serde_json::from_slice(protocol_bytes).unwrap();
    assert_eq!(protocol["experiments_per_setting"], EXPERIMENTS);
    assert_eq!(protocol["maximum_total_assessed_tasks"], MAX_TASKS);
    let protocol_digest = format!("{:x}", Sha256::digest(protocol_bytes));
    let started = Instant::now();
    let mut interval_cache = BTreeMap::new();
    let mut uncertainty_cache = BTreeMap::new();
    let mut settings = Vec::new();
    let null = [
        [100, 1_900, 8_000],
        [500, 1_500, 8_000],
        [2_000, 3_000, 5_000],
        [3_300, 3_400, 3_300],
        [5_000, 3_000, 2_000],
        [8_000, 1_500, 500],
        [9_500, 400, 100],
        [9_900, 90, 10],
    ];
    let nonnull = [
        ([2_000, 3_000, 5_000], [3_000, 3_000, 4_000]),
        ([5_000, 3_000, 2_000], [3_000, 3_000, 4_000]),
        ([100, 1_900, 8_000], [500, 1_500, 8_000]),
        ([9_500, 400, 100], [9_900, 90, 10]),
    ];
    let sizes = [
        (1, 255),
        (2, 2),
        (5, 20),
        (20, 5),
        (20, 20),
        (64, 128),
        (128, 64),
    ];
    let removals = [0u16, 1_000, 5_000];

    for (probability_index, probabilities) in null.into_iter().enumerate() {
        for (size_index, sizes) in sizes.into_iter().enumerate() {
            for (removal_index, removal) in removals.into_iter().enumerate() {
                settings.push(run_setting(
                    "null",
                    0,
                    probability_index as u32,
                    probabilities,
                    probabilities,
                    size_index as u32,
                    sizes,
                    removal_index as u32,
                    removal,
                    &mut interval_cache,
                    &mut uncertainty_cache,
                ));
            }
        }
    }
    for (probability_index, (first, second)) in nonnull.into_iter().enumerate() {
        for (size_index, sizes) in sizes.into_iter().enumerate() {
            for (removal_index, removal) in removals.into_iter().enumerate() {
                settings.push(run_setting(
                    "nonnull",
                    1,
                    probability_index as u32,
                    first,
                    second,
                    size_index as u32,
                    sizes,
                    removal_index as u32,
                    removal,
                    &mut interval_cache,
                    &mut uncertainty_cache,
                ));
            }
        }
    }
    assert_eq!(settings.len(), 252);
    let passing = settings
        .iter()
        .filter(|setting| setting.family_noncoverage_interval_millionths[1] <= 50_000)
        .count() as u32;
    let admitted = passing == settings.len() as u32;
    let artifact = CalibrationArtifact {
        schema_version: 1,
        candidate: "exact_binomial_components_bonferroni_v1",
        qualified_for_saved_specifications: false,
        frozen_protocol_fixture_sha256: protocol_digest,
        experiments_per_setting: EXPERIMENTS,
        elapsed_millis_observed: started.elapsed().as_millis(),
        admission: AdmissionResult {
            all_frozen_settings_completed: true,
            settings_total: settings.len() as u32,
            settings_passing_noncoverage_bound: passing,
            admitted,
            reason: if admitted {
                "candidate_requires_independent_review_before_product_admission"
            } else {
                "one_or_more_frozen_settings_failed_noncoverage_bound"
            },
        },
        settings,
    };
    std::fs::write(output, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn run_setting(
    family: &'static str,
    family_id: u8,
    probability_index: u32,
    first_probability_bps: [u16; 3],
    second_probability_bps: [u16; 3],
    size_index: u32,
    sizes: (usize, usize),
    removal_index: u32,
    removal_bps: u16,
    interval_cache: &mut BTreeMap<(usize, usize), (u32, u32)>,
    uncertainty_cache: &mut BTreeMap<u32, (u32, u32)>,
) -> SettingResult {
    let mut failures = 0u32;
    let mut exclusions = 0u32;
    let mut below_support = 0u32;
    let mut imprecise = 0u32;
    let mut boundary = 0u32;
    let mut widths: [Vec<i64>; 3] = std::array::from_fn(|_| Vec::new());
    for experiment in 0..EXPERIMENTS {
        let first = sample_counts(SampleKey {
            family_id,
            probability_index,
            size_index,
            removal_index,
            experiment,
            cohort: 0,
            size: sizes.0,
            probabilities: first_probability_bps,
            removal_bps,
        });
        let second = sample_counts(SampleKey {
            family_id,
            probability_index,
            size_index,
            removal_index,
            experiment,
            cohort: 1,
            size: sizes.1,
            probabilities: second_probability_bps,
            removal_bps,
        });
        let mut covered = true;
        let mut any_exclusion = false;
        for outcome in 0..3 {
            if first.total < 2 || second.total < 2 {
                below_support += 1;
                continue;
            }
            let first_interval = *interval_cache
                .entry((first.outcomes[outcome], first.total))
                .or_insert_with(|| {
                    exact_component_interval(first.outcomes[outcome], first.total).unwrap()
                });
            let second_interval = *interval_cache
                .entry((second.outcomes[outcome], second.total))
                .or_insert_with(|| {
                    exact_component_interval(second.outcomes[outcome], second.total).unwrap()
                });
            let lower = i64::from(second_interval.0) - i64::from(first_interval.1);
            let upper = i64::from(second_interval.1) - i64::from(first_interval.0);
            let truth = i64::from(second_probability_bps[outcome]) * 100
                - i64::from(first_probability_bps[outcome]) * 100;
            covered &= lower <= truth && truth <= upper;
            let width = upper - lower;
            widths[outcome].push(width);
            if width > MAXIMUM_WIDTH {
                imprecise += 1;
            } else if width == MAXIMUM_WIDTH || lower == 0 || upper == 0 {
                boundary += 1;
            } else if upper < 0 || lower > 0 {
                any_exclusion = true;
            }
        }
        failures += u32::from(!covered);
        exclusions += u32::from(any_exclusion);
    }
    let uncertainty = *uncertainty_cache.entry(failures).or_insert_with(|| {
        exact_binomial_interval(failures as usize, EXPERIMENTS as usize, 40).unwrap()
    });
    SettingResult {
        family,
        probability_index,
        first_probability_bps,
        second_probability_bps,
        size_index,
        first_size: sizes.0,
        second_size: sizes.1,
        removal_index,
        assessed_removal_bps: removal_bps,
        family_noncoverage_count: failures,
        family_noncoverage_interval_millionths: [uncertainty.0, uncertainty.1],
        experiments_with_any_zero_exclusion: exclusions,
        below_minimum_support_contrasts: below_support,
        insufficient_precision_contrasts: imprecise,
        indeterminate_boundary_contrasts: boundary,
        interval_width_millionths: widths.map(width_summary),
    }
}

struct SampleKey {
    family_id: u8,
    probability_index: u32,
    size_index: u32,
    removal_index: u32,
    experiment: u32,
    cohort: u8,
    size: usize,
    probabilities: [u16; 3],
    removal_bps: u16,
}

fn sample_counts(key: SampleKey) -> Counts {
    let mut counts = Counts::default();
    for draw in 0..key.size {
        let bucket = sample_bucket(&key, draw as u16, false);
        let outcome = if bucket < u64::from(key.probabilities[0]) {
            0
        } else if bucket < u64::from(key.probabilities[0]) + u64::from(key.probabilities[1]) {
            1
        } else {
            2
        };
        if key.removal_bps != 0
            && sample_bucket(&key, draw as u16, true) < u64::from(key.removal_bps)
        {
            continue;
        }
        counts.total += 1;
        counts.outcomes[outcome] += 1;
    }
    counts
}

fn sample_bucket(key: &SampleKey, draw: u16, removal: bool) -> u64 {
    let limit = (u128::from(u64::MAX) + 1) / 10_000 * 10_000;
    for retry in 0u32.. {
        let mut hash = Sha256::new();
        hash.update(b"trace-commons-exact-component-qualification-v1\0");
        if removal {
            hash.update(b"removal");
        }
        hash.update([key.family_id]);
        hash.update(key.probability_index.to_be_bytes());
        hash.update(key.size_index.to_be_bytes());
        hash.update(key.removal_index.to_be_bytes());
        hash.update(key.experiment.to_be_bytes());
        hash.update([key.cohort]);
        hash.update(draw.to_be_bytes());
        hash.update(retry.to_be_bytes());
        let bytes = hash.finalize();
        let value = u64::from_be_bytes(bytes[..8].try_into().unwrap()) as u128;
        if value < limit {
            return (value % 10_000) as u64;
        }
    }
    unreachable!()
}

fn width_summary(mut values: Vec<i64>) -> WidthSummary {
    values.sort_unstable();
    WidthSummary {
        observed: values.len() as u32,
        p50: percentile(&values, 50),
        p90: percentile(&values, 90),
        p95: percentile(&values, 95),
        maximum: values.last().copied(),
    }
}

fn percentile(values: &[i64], percent: usize) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let rank = (values.len() * percent).div_ceil(100).saturating_sub(1);
    values.get(rank).copied()
}

#[test]
fn calibration_sampler_and_width_summary_are_stable() {
    let counts = sample_counts(SampleKey {
        family_id: 0,
        probability_index: 0,
        size_index: 0,
        removal_index: 0,
        experiment: 0,
        cohort: 0,
        size: 255,
        probabilities: [100, 1_900, 8_000],
        removal_bps: 0,
    });
    assert_eq!(counts.total, 255);
    assert_eq!(counts.outcomes, [2, 40, 213]);
    let summary = width_summary(vec![9, 1, 5, 3, 7]);
    assert_eq!(
        (summary.p50, summary.p90, summary.p95, summary.maximum),
        (Some(5), Some(9), Some(9), Some(9))
    );
}
