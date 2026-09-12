use std::collections::BTreeMap;
use std::io::Write;
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
    historical_frozen_error_interpretation: &'static str,
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
    evaluated_interval_experiments: u32,
    unevaluated_below_support_experiments: u32,
    conditional_interval_noncoverage_among_evaluated_millionths: Option<[u32; 2]>,
    experiments_with_any_zero_exclusion: u32,
    any_zero_exclusion_interval_millionths: [u32; 2],
    below_minimum_support_contrasts: u32,
    below_minimum_support_rate_bps: u16,
    insufficient_precision_contrasts: u32,
    insufficient_precision_rate_bps: u16,
    indeterminate_boundary_contrasts: u32,
    indeterminate_boundary_rate_bps: u16,
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

#[derive(Clone, Copy, Default)]
struct Counts {
    total: usize,
    outcomes: [usize; 3],
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CandidateDecision {
    InsufficientPrecision,
    IndeterminateBoundary,
    ExcludesZero,
    IncludesZero,
}

#[derive(Serialize)]
struct CandidateContrast {
    lower_millionths: i64,
    upper_millionths: i64,
    width_millionths: i64,
    decision: CandidateDecision,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CandidateEvaluation {
    Supported { contrasts: [CandidateContrast; 3] },
    SuppressedBelowMinimumCohortSupport,
}

fn combine_candidate_intervals(first: (u32, u32), second: (u32, u32)) -> CandidateContrast {
    let lower = i64::from(second.0) - i64::from(first.1);
    let upper = i64::from(second.1) - i64::from(first.0);
    let width = upper - lower;
    let decision = if width > MAXIMUM_WIDTH {
        CandidateDecision::InsufficientPrecision
    } else if width == MAXIMUM_WIDTH || lower == 0 || upper == 0 {
        CandidateDecision::IndeterminateBoundary
    } else if upper < 0 || lower > 0 {
        CandidateDecision::ExcludesZero
    } else {
        CandidateDecision::IncludesZero
    };
    CandidateContrast {
        lower_millionths: lower,
        upper_millionths: upper,
        width_millionths: width,
        decision,
    }
}

fn evaluate_candidate_counts(
    first: &Counts,
    second: &Counts,
) -> anyhow::Result<CandidateEvaluation> {
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
        return Err(anyhow::anyhow!("insights-comparison-estimator-invalid"));
    }
    if first.total < 2 || second.total < 2 {
        return Ok(CandidateEvaluation::SuppressedBelowMinimumCohortSupport);
    }
    let mut contrasts = Vec::with_capacity(3);
    for outcome in 0..3 {
        contrasts.push(combine_candidate_intervals(
            exact_component_interval(first.outcomes[outcome], first.total)?,
            exact_component_interval(second.outcomes[outcome], second.total)?,
        ));
    }
    Ok(CandidateEvaluation::Supported {
        contrasts: contrasts
            .try_into()
            .map_err(|_| anyhow::anyhow!("insights-comparison-estimator-invalid"))?,
    })
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
    assert_frozen_protocol(&protocol, &null, &nonnull, &sizes, &removals);

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
        historical_frozen_error_interpretation: "unconditional_product_rule_error_over_all_experiments__below_support_emits_no_interval_and_counts_as_no_error",
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
    atomic_write(
        &std::path::PathBuf::from(output),
        &serde_json::to_vec_pretty(&artifact).unwrap(),
    );
}

#[test]
#[ignore = "derives complete reporting from the immutable 9d41d5b4 raw artifact"]
fn derive_complete_report_from_raw_artifact() {
    let input = std::env::var_os("TRACE_COMMONS_COMPARISON_CALIBRATION_RAW")
        .expect("TRACE_COMMONS_COMPARISON_CALIBRATION_RAW is required");
    let output = std::env::var_os("TRACE_COMMONS_COMPARISON_CALIBRATION_OUTPUT")
        .expect("TRACE_COMMONS_COMPARISON_CALIBRATION_OUTPUT is required");
    let raw = std::fs::read(input).unwrap();
    let mut artifact: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    validate_raw_artifact(&artifact);
    artifact["executed_runner_commit"] = serde_json::json!("9d41d5b4");
    artifact["raw_artifact_sha256"] = serde_json::json!(format!("{:x}", Sha256::digest(&raw)));
    artifact["historical_frozen_error_interpretation"] = serde_json::json!(
        "unconditional_product_rule_error_over_all_experiments__below_support_emits_no_interval_and_counts_as_no_error"
    );
    let settings = artifact["settings"].as_array_mut().unwrap();
    let mut cache = BTreeMap::new();
    for setting in settings.iter_mut() {
        let object = setting.as_object_mut().unwrap();
        let failures = object
            .get("family_noncoverage_count")
            .unwrap()
            .as_u64()
            .unwrap() as u32;
        let exclusions = object["experiments_with_any_zero_exclusion"]
            .as_u64()
            .unwrap() as u32;
        let below = object["below_minimum_support_contrasts"].as_u64().unwrap() as u32;
        let imprecise = object["insufficient_precision_contrasts"].as_u64().unwrap() as u32;
        let boundary = object["indeterminate_boundary_contrasts"].as_u64().unwrap() as u32;
        assert_eq!(below % 3, 0);
        let unevaluated = below / 3;
        let evaluated = EXPERIMENTS - unevaluated;
        let failure_interval = if evaluated == 0 {
            None
        } else {
            let interval = *cache.entry((failures, evaluated)).or_insert_with(|| {
                exact_binomial_interval(failures as usize, evaluated as usize, 40).unwrap()
            });
            Some([interval.0, interval.1])
        };
        let exclusion_interval = *cache.entry((exclusions, EXPERIMENTS)).or_insert_with(|| {
            exact_binomial_interval(exclusions as usize, EXPERIMENTS as usize, 40).unwrap()
        });
        object.insert(
            "evaluated_interval_experiments".into(),
            serde_json::json!(evaluated),
        );
        object.insert(
            "unevaluated_below_support_experiments".into(),
            serde_json::json!(unevaluated),
        );
        object.insert(
            "conditional_interval_noncoverage_among_evaluated_millionths".into(),
            serde_json::json!(failure_interval),
        );
        object.insert(
            "any_zero_exclusion_interval_millionths".into(),
            serde_json::json!([exclusion_interval.0, exclusion_interval.1]),
        );
        object.insert(
            "below_minimum_support_rate_bps".into(),
            serde_json::json!(rate_bps(below, EXPERIMENTS * 3)),
        );
        object.insert(
            "insufficient_precision_rate_bps".into(),
            serde_json::json!(rate_bps(imprecise, EXPERIMENTS * 3)),
        );
        object.insert(
            "indeterminate_boundary_rate_bps".into(),
            serde_json::json!(rate_bps(boundary, EXPERIMENTS * 3)),
        );
    }
    artifact["derivation"] = serde_json::json!({
        "conditional_interval_noncoverage_is_diagnostic_only": true,
        "historical_frozen_admission_preserved": true,
        "product_admission": false,
        "reason": "pending_independent_method_artifact_and_runtime_review"
    });
    atomic_write(
        &std::path::PathBuf::from(output),
        &serde_json::to_vec_pretty(&artifact).unwrap(),
    );
}

fn validate_raw_artifact(artifact: &serde_json::Value) {
    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(
        artifact["candidate"],
        "exact_binomial_components_bonferroni_v1"
    );
    assert_eq!(artifact["qualified_for_saved_specifications"], false);
    assert_eq!(artifact["experiments_per_setting"], EXPERIMENTS);
    assert_eq!(
        artifact["frozen_protocol_fixture_sha256"],
        "d257605993aaa94220ce31144203c0c05a28ff6c33cc129d678f90a8986e26c8"
    );
    assert_eq!(artifact["admission"]["all_frozen_settings_completed"], true);
    assert_eq!(artifact["admission"]["settings_total"], 252);
    let protocol: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../fixtures/insights/comparison-estimator/exact-component-candidate-v1.json"
    ))
    .unwrap();
    let settings = artifact["settings"].as_array().unwrap();
    assert_eq!(settings.len(), 252);
    let mut identities = std::collections::BTreeSet::new();
    let mut passing = 0u32;
    for setting in settings {
        let family = setting["family"].as_str().unwrap();
        let probability = setting["probability_index"].as_u64().unwrap() as usize;
        let size = setting["size_index"].as_u64().unwrap() as usize;
        let removal = setting["removal_index"].as_u64().unwrap() as usize;
        assert!(size < 7 && removal < 3);
        assert!((family == "null" && probability < 8) || (family == "nonnull" && probability < 4));
        assert!(identities.insert((family, probability, size, removal)));
        assert_eq!(
            setting["assessed_removal_bps"],
            protocol["assessed_only_removal_bps"][removal]
        );
        assert_eq!(
            setting["first_size"],
            protocol["cohort_size_pairs"][size][0]
        );
        assert_eq!(
            setting["second_size"],
            protocol["cohort_size_pairs"][size][1]
        );
        let expected = if family == "null" {
            &protocol["null_probability_bps"][probability]
        } else {
            assert_eq!(family, "nonnull");
            &protocol["nonnull_probability_pairs_bps"][probability][0]
        };
        assert_eq!(&setting["first_probability_bps"], expected);
        let expected = if family == "null" {
            &protocol["null_probability_bps"][probability]
        } else {
            &protocol["nonnull_probability_pairs_bps"][probability][1]
        };
        assert_eq!(&setting["second_probability_bps"], expected);
        let failures = setting["family_noncoverage_count"].as_u64().unwrap();
        let raw_interval = setting["family_noncoverage_interval_millionths"]
            .as_array()
            .unwrap();
        assert_eq!(raw_interval.len(), 2);
        let raw_lower = raw_interval[0].as_u64().unwrap();
        let raw_upper = raw_interval[1].as_u64().unwrap();
        assert!(raw_lower <= raw_upper && raw_upper <= 1_000_000);
        passing += u32::from(raw_upper <= 50_000);
        let exclusions = setting["experiments_with_any_zero_exclusion"]
            .as_u64()
            .unwrap();
        let below = setting["below_minimum_support_contrasts"].as_u64().unwrap();
        let imprecise = setting["insufficient_precision_contrasts"]
            .as_u64()
            .unwrap();
        let boundary = setting["indeterminate_boundary_contrasts"]
            .as_u64()
            .unwrap();
        assert!(failures <= u64::from(EXPERIMENTS));
        assert!(below <= u64::from(EXPERIMENTS * 3) && below % 3 == 0);
        assert!(imprecise <= u64::from(EXPERIMENTS * 3));
        assert!(boundary <= u64::from(EXPERIMENTS * 3));
        let evaluated = u64::from(EXPERIMENTS) - below / 3;
        assert!(failures <= evaluated);
        assert!(exclusions <= evaluated);
        assert!(imprecise + boundary <= 3 * evaluated);
        let widths = setting["interval_width_millionths"].as_array().unwrap();
        assert_eq!(widths.len(), 3);
        for width in widths {
            assert_eq!(width["observed"], evaluated);
            let quantiles = ["p50", "p90", "p95", "maximum"].map(|name| width[name].as_i64());
            if evaluated == 0 {
                assert_eq!(quantiles, [None; 4]);
            } else {
                let [Some(p50), Some(p90), Some(p95), Some(maximum)] = quantiles else {
                    panic!("evaluated width summary requires every quantile")
                };
                assert!(0 <= p50 && p50 <= p90 && p90 <= p95 && p95 <= maximum);
                assert!(maximum <= 2_000_000);
            }
        }
    }
    let expected = [("null", 8usize), ("nonnull", 4usize)]
        .into_iter()
        .flat_map(|(family, probabilities)| {
            (0..probabilities).flat_map(move |probability| {
                (0..7).flat_map(move |size| {
                    (0..3).map(move |removal| (family, probability, size, removal))
                })
            })
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities, expected);
    assert_eq!(
        artifact["admission"]["settings_passing_noncoverage_bound"],
        passing
    );
    assert_eq!(artifact["admission"]["admitted"], passing == 252);
}

fn assert_frozen_protocol(
    protocol: &serde_json::Value,
    null: &[[u16; 3]; 8],
    nonnull: &[([u16; 3], [u16; 3]); 4],
    sizes: &[(usize, usize); 7],
    removals: &[u16; 3],
) {
    assert_eq!(
        protocol["candidate"],
        "exact_binomial_components_bonferroni_v1"
    );
    assert_eq!(protocol["qualified_for_saved_specifications"], false);
    assert_eq!(protocol["maximum_interval_width_bps"], 5_000);
    assert_eq!(protocol["minimum_product_support_per_cohort"], 2);
    assert_eq!(
        protocol["null_probability_bps"],
        serde_json::to_value(null).unwrap()
    );
    let nonnull = nonnull
        .iter()
        .map(|(first, second)| [first, second])
        .collect::<Vec<_>>();
    assert_eq!(
        protocol["nonnull_probability_pairs_bps"],
        serde_json::to_value(nonnull).unwrap()
    );
    assert_eq!(
        protocol["cohort_size_pairs"],
        serde_json::to_value(sizes).unwrap()
    );
    assert_eq!(
        protocol["assessed_only_removal_bps"],
        serde_json::to_value(removals).unwrap()
    );
    let qualification = &protocol["qualification_protocol"];
    assert_eq!(qualification["prf"], "sha256");
    assert!(qualification["outcome_seed_bytes"].as_str().is_some());
    assert!(qualification["removal_seed_bytes"].as_str().is_some());
    assert_eq!(
        qualification["aggregation"],
        "none_for_admission__each_frozen_setting_must_pass_its_own_upper_bound"
    );
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).unwrap();
    file.write_all(bytes).unwrap();
    file.as_file().sync_all().unwrap();
    file.persist_noclobber(path).unwrap();
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
    uncertainty_cache: &mut BTreeMap<(u32, u32), (u32, u32)>,
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
            let contrast = combine_candidate_intervals(first_interval, second_interval);
            let lower = contrast.lower_millionths;
            let upper = contrast.upper_millionths;
            let truth = i64::from(second_probability_bps[outcome]) * 100
                - i64::from(first_probability_bps[outcome]) * 100;
            covered &= lower <= truth && truth <= upper;
            let width = contrast.width_millionths;
            widths[outcome].push(width);
            match contrast.decision {
                CandidateDecision::InsufficientPrecision => imprecise += 1,
                CandidateDecision::IndeterminateBoundary => boundary += 1,
                CandidateDecision::ExcludesZero => any_exclusion = true,
                CandidateDecision::IncludesZero => {}
            }
        }
        failures += u32::from(!covered);
        exclusions += u32::from(any_exclusion);
    }
    assert_eq!(below_support % 3, 0);
    let unevaluated = below_support / 3;
    let evaluated = EXPERIMENTS - unevaluated;
    let conditional_uncertainty = (evaluated != 0).then(|| {
        *uncertainty_cache
            .entry((failures, evaluated))
            .or_insert_with(|| {
                exact_binomial_interval(failures as usize, evaluated as usize, 40).unwrap()
            })
    });
    let uncertainty = *uncertainty_cache
        .entry((failures, EXPERIMENTS))
        .or_insert_with(|| {
            exact_binomial_interval(failures as usize, EXPERIMENTS as usize, 40).unwrap()
        });
    let exclusion_uncertainty = *uncertainty_cache
        .entry((exclusions, EXPERIMENTS))
        .or_insert_with(|| {
            exact_binomial_interval(exclusions as usize, EXPERIMENTS as usize, 40).unwrap()
        });
    let contrast_total = EXPERIMENTS * 3;
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
        evaluated_interval_experiments: evaluated,
        unevaluated_below_support_experiments: unevaluated,
        conditional_interval_noncoverage_among_evaluated_millionths: conditional_uncertainty
            .map(|interval| [interval.0, interval.1]),
        experiments_with_any_zero_exclusion: exclusions,
        any_zero_exclusion_interval_millionths: [exclusion_uncertainty.0, exclusion_uncertainty.1],
        below_minimum_support_contrasts: below_support,
        below_minimum_support_rate_bps: rate_bps(below_support, contrast_total),
        insufficient_precision_contrasts: imprecise,
        insufficient_precision_rate_bps: rate_bps(imprecise, contrast_total),
        indeterminate_boundary_contrasts: boundary,
        indeterminate_boundary_rate_bps: rate_bps(boundary, contrast_total),
        interval_width_millionths: widths.map(width_summary),
    }
}

fn rate_bps(count: u32, total: u32) -> u16 {
    u16::try_from((u64::from(count) * 10_000 + u64::from(total / 2)) / u64::from(total)).unwrap()
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

fn synthetic_raw_artifact() -> serde_json::Value {
    let protocol: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../fixtures/insights/comparison-estimator/exact-component-candidate-v1.json"
    ))
    .unwrap();
    let mut settings = Vec::new();
    for (family, probabilities) in [("null", 8usize), ("nonnull", 4usize)] {
        for probability in 0..probabilities {
            for size in 0..7usize {
                for removal in 0..3usize {
                    let below = u32::from(size == 0) * EXPERIMENTS * 3;
                    let evaluated = u64::from(EXPERIMENTS - below / 3);
                    let width = if evaluated == 0 {
                        serde_json::json!({"observed": 0, "p50": null, "p90": null, "p95": null, "maximum": null})
                    } else {
                        serde_json::json!({"observed": evaluated, "p50": 100, "p90": 200, "p95": 300, "maximum": 400})
                    };
                    let (first, second) = if family == "null" {
                        let value = protocol["null_probability_bps"][probability].clone();
                        (value.clone(), value)
                    } else {
                        (
                            protocol["nonnull_probability_pairs_bps"][probability][0].clone(),
                            protocol["nonnull_probability_pairs_bps"][probability][1].clone(),
                        )
                    };
                    settings.push(serde_json::json!({
                        "family": family,
                        "probability_index": probability,
                        "first_probability_bps": first,
                        "second_probability_bps": second,
                        "size_index": size,
                        "first_size": protocol["cohort_size_pairs"][size][0],
                        "second_size": protocol["cohort_size_pairs"][size][1],
                        "removal_index": removal,
                        "assessed_removal_bps": protocol["assessed_only_removal_bps"][removal],
                        "family_noncoverage_count": 0,
                        "family_noncoverage_interval_millionths": [0, 1_000],
                        "experiments_with_any_zero_exclusion": 0,
                        "below_minimum_support_contrasts": below,
                        "insufficient_precision_contrasts": 0,
                        "indeterminate_boundary_contrasts": 0,
                        "interval_width_millionths": [width.clone(), width.clone(), width]
                    }));
                }
            }
        }
    }
    serde_json::json!({
        "schema_version": 1,
        "candidate": "exact_binomial_components_bonferroni_v1",
        "qualified_for_saved_specifications": false,
        "frozen_protocol_fixture_sha256": "d257605993aaa94220ce31144203c0c05a28ff6c33cc129d678f90a8986e26c8",
        "experiments_per_setting": EXPERIMENTS,
        "settings": settings,
        "admission": {
            "all_frozen_settings_completed": true,
            "settings_total": 252,
            "settings_passing_noncoverage_bound": 252,
            "admitted": true
        }
    })
}

fn rejected_by_raw_validator(value: serde_json::Value) {
    assert!(std::panic::catch_unwind(|| validate_raw_artifact(&value)).is_err());
}

#[test]
fn raw_report_validator_rejects_missing_duplicate_and_out_of_range_settings() {
    let valid = synthetic_raw_artifact();
    validate_raw_artifact(&valid);
    let mut missing = valid.clone();
    missing["settings"].as_array_mut().unwrap().pop();
    rejected_by_raw_validator(missing);
    let mut duplicate = valid.clone();
    let first = duplicate["settings"][0].clone();
    duplicate["settings"].as_array_mut().unwrap()[251] = first;
    rejected_by_raw_validator(duplicate);
    let mut out_of_range = valid;
    out_of_range["settings"][0]["probability_index"] = serde_json::json!(8);
    out_of_range["settings"][0]["first_probability_bps"] = serde_json::Value::Null;
    out_of_range["settings"][0]["second_probability_bps"] = serde_json::Value::Null;
    rejected_by_raw_validator(out_of_range);
}

#[test]
fn raw_report_validator_rejects_width_and_counter_corruption() {
    let valid = synthetic_raw_artifact();
    let mut widths = valid.clone();
    widths["settings"][21]["interval_width_millionths"] = serde_json::json!([]);
    rejected_by_raw_validator(widths);
    let mut exclusions = valid.clone();
    exclusions["settings"][0]["experiments_with_any_zero_exclusion"] = serde_json::json!(1);
    rejected_by_raw_validator(exclusions);
    let mut overlap = valid.clone();
    overlap["settings"][21]["insufficient_precision_contrasts"] = serde_json::json!(30_000);
    overlap["settings"][21]["indeterminate_boundary_contrasts"] = serde_json::json!(1);
    rejected_by_raw_validator(overlap);
    let mut admission = valid;
    admission["admission"]["settings_passing_noncoverage_bound"] = serde_json::json!(251);
    rejected_by_raw_validator(admission);
}

#[test]
fn atomic_report_publication_does_not_replace_existing_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("artifact.json");
    atomic_write(&path, b"first");
    assert!(std::panic::catch_unwind(|| atomic_write(&path, b"second")).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"first");
}

#[derive(Serialize)]
struct RuntimeObservation<'a> {
    schema_version: u32,
    case: &'a str,
    first_total: usize,
    first_outcomes: [usize; 3],
    second_total: usize,
    second_outcomes: [usize; 3],
    candidate_elapsed_nanos: u128,
    evaluation: CandidateEvaluation,
}

#[test]
#[ignore = "measures one fresh-process exact candidate evaluation"]
fn measure_exact_candidate_evaluation() {
    let case = std::env::var("TRACE_COMMONS_EXACT_EVALUATION_CASE")
        .expect("TRACE_COMMONS_EXACT_EVALUATION_CASE is required");
    let output = std::env::var_os("TRACE_COMMONS_EXACT_EVALUATION_OUTPUT")
        .expect("TRACE_COMMONS_EXACT_EVALUATION_OUTPUT is required");
    let (first, second, expected_supported) = match case.as_str() {
        "balanced_boundary" => (
            Counts {
                total: 128,
                outcomes: [128, 0, 0],
            },
            Counts {
                total: 128,
                outcomes: [128, 0, 0],
            },
            true,
        ),
        "balanced_interior" => (
            Counts {
                total: 128,
                outcomes: [43, 43, 42],
            },
            Counts {
                total: 128,
                outcomes: [43, 43, 42],
            },
            true,
        ),
        "imbalanced_supported" => (
            Counts {
                total: 254,
                outcomes: [127, 64, 63],
            },
            Counts {
                total: 2,
                outcomes: [1, 1, 0],
            },
            true,
        ),
        "imbalanced_suppressed" => (
            Counts {
                total: 255,
                outcomes: [128, 64, 63],
            },
            Counts {
                total: 1,
                outcomes: [1, 0, 0],
            },
            false,
        ),
        _ => panic!("unsupported exact evaluation case"),
    };
    let first = std::hint::black_box(first);
    let second = std::hint::black_box(second);
    let started = Instant::now();
    let evaluation = evaluate_candidate_counts(&first, &second).unwrap();
    let candidate_elapsed_nanos = started.elapsed().as_nanos();
    assert_eq!(
        matches!(evaluation, CandidateEvaluation::Supported { .. }),
        expected_supported
    );
    let observation = RuntimeObservation {
        schema_version: 1,
        case: &case,
        first_total: first.total,
        first_outcomes: first.outcomes,
        second_total: second.total,
        second_outcomes: second.outcomes,
        candidate_elapsed_nanos,
        evaluation,
    };
    atomic_write(
        std::path::Path::new(&output),
        &serde_json::to_vec_pretty(&observation).unwrap(),
    );
}
