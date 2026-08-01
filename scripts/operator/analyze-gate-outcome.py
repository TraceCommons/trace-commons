#!/usr/bin/env python3
"""Measure whether gate signals discriminate failed from successful traces.

Expected CSV schema (header required, one row per scored submission):

    tenant_id,submission_id,decided_at,credit_quality_micros,perplexity_micros,tail_fraction_micros,novelty_score_micros,task_success

`task_success` must be `success`, `partial`, `failure`, or `unknown`.
Partial and unknown rows are counted and excluded from both AUC arms.
`credit_quality_micros` is optional, nullable on rows predating its backfill,
and analyzed first as the primary combined graded-credit score. Null values are
excluded only from that primary analysis. An optional final `length` column
enables the trace-length covariate. For
`--label=human_correction`, append a `human_correction` column containing
`true`, `false`, `partial`, or `unknown`; true is the failure arm.

At least 10 independent tenant clusters are required by default because a
cluster bootstrap with fewer resampling units cannot support a reliable
estimate of between-cluster uncertainty. Override with `--min-clusters`
only when a different threshold is justified before examining outcomes.
The detection verdict comes from a cluster-level label permutation test. Each
permutation moves a tenant's complete label vector to another tenant while
leaving scores fixed. Short donor vectors wrap to fill longer receiving
clusters. This preserves within-tenant label clustering under the null. The
cluster-bootstrap interval remains an interval at the observed effect. Label
ICC, size-weighted mean cluster size, and label design effect are descriptive
label-clustering diagnostics only.

No minimum detectable effect or achieved power is claimed by default. Supplying
`--alternative-auc` runs a Monte Carlo power simulation: tenant clusters are
resampled whole, labels retain their cluster composition, and independent
Gaussian scores are shifted by the amount whose population AUC equals the
prespecified alternative. Each simulated sample is evaluated by the same
cluster-level label permutation test.

PostgreSQL export query for the default schema:

    COPY (
      WITH latest_gate AS (
        SELECT DISTINCT ON (tenant_id, submission_id)
          tenant_id, submission_id, decided_at, perplexity_micros,
          tail_fraction_micros, novelty_score_micros
        FROM trace_gate_decisions
        ORDER BY tenant_id, submission_id, decided_at DESC, decision_id DESC
      ),
      latest_outcome AS (
        SELECT DISTINCT ON (tenant_id, submission_id)
          tenant_id, submission_id, task_success
        FROM trace_derived_records
        WHERE status = 'current' AND task_success IS NOT NULL
        ORDER BY tenant_id, submission_id, updated_at DESC, derived_id DESC
      )
      SELECT md5(g.tenant_id) AS tenant_id, g.submission_id, g.decided_at,
             g.credit_quality_micros, g.perplexity_micros, g.tail_fraction_micros,
             g.novelty_score_micros, o.task_success
      FROM latest_gate g
      JOIN latest_outcome o USING (tenant_id, submission_id)
      ORDER BY g.tenant_id, g.submission_id
    ) TO STDOUT WITH (FORMAT CSV, HEADER);

To include the available length proxy, add `event_count AS length` to
`latest_outcome` and select `o.length` last. The server does not persist
`human_correction`; that alternative label requires a separate, consented
export reduced to the categorical values above.

Usage:

    python3 scripts/operator/analyze-gate-outcome.py \
        --input=gate-outcome.csv \
        --label=task_success \
        --min-clusters=10 \
        --min-per-class=125 \
        --bootstrap=400 \
        --permutations=400 \
        --alpha=0.05 \
        --seed=12345

Pure-stdlib (argparse + csv + datetime + math + random + statistics).
"""

from __future__ import annotations

import argparse
import csv
import math
import random
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from statistics import NormalDist, stdev
from typing import Optional


REQUIRED_COLUMNS = (
    "tenant_id",
    "submission_id",
    "decided_at",
    "perplexity_micros",
    "tail_fraction_micros",
    "novelty_score_micros",
    "task_success",
)
OPTIONAL_COLUMNS = (
    "credit_quality_micros",
    "human_correction",
    "length",
)
SIGNALS = {
    "perplexity": "perplexity_micros",
    "tail_fraction": "tail_fraction_micros",
    "novelty": "novelty_score_micros",
}
FAILURE = 1
SUCCESS = 0


class AnalyzeGateOutcomeError(Exception):
    """Operator-actionable input or argument failure."""


class OperatorArgumentParser(argparse.ArgumentParser):
    """Route argparse failures through the operator error convention."""

    def error(self, message: str) -> None:
        raise AnalyzeGateOutcomeError(f"invalid_arguments detail={message}")


@dataclass(frozen=True)
class Observation:
    tenant_id: str
    submission_id: str
    label: int
    decided_at: float
    values: dict[str, float]


@dataclass(frozen=True)
class LoadedObservations:
    observations: list[Observation]
    excluded: dict[str, int]
    has_length: bool
    has_credit_quality: bool
    credit_quality_nulls: int
    total_rows: int


@dataclass(frozen=True)
class BootstrapResult:
    lower: Optional[float]
    upper: Optional[float]
    standard_error: Optional[float]
    valid_count: int
    undefined_count: int
    iterations: int

    @property
    def valid_fraction(self) -> float:
        return self.valid_count / self.iterations

    @property
    def interval(self) -> tuple[float, float]:
        if self.lower is None or self.upper is None:
            raise AnalyzeGateOutcomeError("bootstrap_produced_no_estimates")
        return self.lower, self.upper


@dataclass(frozen=True)
class PermutationResult:
    p_value: float
    extreme_count: int
    valid_count: int
    undefined_count: int
    iterations: int


@dataclass(frozen=True)
class PowerResult:
    achieved_power: float
    rejection_count: int
    valid_count: int
    undefined_count: int
    iterations: int


@dataclass(frozen=True)
class SignalAnalysis:
    name: str
    role: str
    included: int
    null_excluded: int
    auc: float
    naive: BootstrapResult
    clustered: BootstrapResult
    permutation: PermutationResult
    power: Optional[PowerResult]
    status: str


def discrimination_auc(positive: list[float], negative: list[float]) -> float:
    """Mann–Whitney AUC; ties score 0.5 and an empty arm returns 0.5.

    This is equivalent to `gate_calibrate::bakeoff_metrics::
    discrimination_auc`: positive values are the Rust function's `novel`
    arm and negative values are its `duplicate` arm.
    """

    if not positive or not negative:
        return 0.5

    ranked = [(value, 1) for value in positive]
    ranked.extend((value, 0) for value in negative)
    ranked.sort(key=lambda item: item[0])

    wins = 0.0
    negatives_before = 0
    index = 0
    while index < len(ranked):
        end = index + 1
        value = ranked[index][0]
        while end < len(ranked) and ranked[end][0] == value:
            end += 1
        positives_tied = sum(label for _, label in ranked[index:end])
        negatives_tied = end - index - positives_tied
        wins += positives_tied * (negatives_before + 0.5 * negatives_tied)
        negatives_before += negatives_tied
        index = end

    return wins / (len(positive) * len(negative))


def intraclass_correlation(
    labels_by_tenant: dict[str, list[int]],
) -> float:
    """Return ICC(1) for binary labels."""

    clusters = [values for values in labels_by_tenant.values() if values]
    tenant_count = len(clusters)
    total_count = sum(len(values) for values in clusters)
    if tenant_count <= 1 or total_count <= tenant_count:
        return 0.0

    cluster_stats = [
        (values, len(values), sum(values) / len(values))
        for values in clusters
    ]
    grand_mean = sum(
        cluster_size * cluster_mean
        for _, cluster_size, cluster_mean in cluster_stats
    ) / total_count
    between_ss = sum(
        cluster_size * (cluster_mean - grand_mean) ** 2
        for _, cluster_size, cluster_mean in cluster_stats
    )
    within_ss = sum(
        sum((value - cluster_mean) ** 2 for value in values)
        for values, _, cluster_mean in cluster_stats
    )
    between_ms = between_ss / (tenant_count - 1)
    within_ms = within_ss / (total_count - tenant_count)
    m0 = (
        total_count
        - sum(len(values) ** 2 for values in clusters) / total_count
    ) / (tenant_count - 1)
    denominator = between_ms + (m0 - 1.0) * within_ms
    if denominator == 0.0:
        return 0.0
    return (between_ms - within_ms) / denominator


def size_weighted_mean_cluster_size(
    labels_by_tenant: dict[str, list[int]],
) -> float:
    """Return sum(m^2) / sum(m) for non-empty tenant clusters."""

    cluster_sizes = [
        len(values) for values in labels_by_tenant.values() if values
    ]
    total_count = sum(cluster_sizes)
    if total_count == 0:
        return 1.0
    return sum(size * size for size in cluster_sizes) / total_count


def design_effect(icc: float, size_weighted_mean: float) -> float:
    return 1.0 + (size_weighted_mean - 1.0) * icc


def percentile(values: list[float], probability: float) -> float:
    if not values:
        raise AnalyzeGateOutcomeError("bootstrap_produced_no_estimates")
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def _scores(
    observations: list[Observation],
    signal: str,
) -> tuple[list[float], list[float]]:
    positive = [
        row.values[signal] for row in observations if row.label == FAILURE
    ]
    negative = [
        row.values[signal] for row in observations if row.label == SUCCESS
    ]
    return positive, negative


def bootstrap_auc_interval(
    observations: list[Observation],
    signal: str,
    *,
    iterations: int,
    seed: int,
    clustered: bool,
) -> BootstrapResult:
    """Bootstrap rows or tenants; reject replicates lacking either class."""

    rng = random.Random(seed)
    estimates: list[float] = []
    undefined_count = 0

    if clustered:
        by_tenant: dict[str, list[Observation]] = {}
        for row in observations:
            by_tenant.setdefault(row.tenant_id, []).append(row)
        tenant_ids = sorted(by_tenant)
        for _ in range(iterations):
            sampled: list[Observation] = []
            for _ in tenant_ids:
                sampled.extend(by_tenant[rng.choice(tenant_ids)])
            positive, negative = _scores(sampled, signal)
            if not positive or not negative:
                undefined_count += 1
                continue
            estimates.append(discrimination_auc(positive, negative))
    else:
        for _ in range(iterations):
            sampled = [rng.choice(observations) for _ in observations]
            positive, negative = _scores(sampled, signal)
            if not positive or not negative:
                undefined_count += 1
                continue
            estimates.append(discrimination_auc(positive, negative))

    lower = percentile(estimates, 0.025) if estimates else None
    upper = percentile(estimates, 0.975) if estimates else None
    standard_error = stdev(estimates) if len(estimates) >= 2 else None
    return BootstrapResult(
        lower=lower,
        upper=upper,
        standard_error=standard_error,
        valid_count=len(estimates),
        undefined_count=undefined_count,
        iterations=iterations,
    )


def rank_values(values: list[float]) -> list[float]:
    """Average ranks for ties, starting at one."""

    indexed = sorted(enumerate(values), key=lambda item: item[1])
    ranks = [0.0] * len(values)
    index = 0
    while index < len(indexed):
        end = index + 1
        value = indexed[index][1]
        while end < len(indexed) and indexed[end][1] == value:
            end += 1
        average_rank = ((index + 1) + end) / 2.0
        for original_index, _ in indexed[index:end]:
            ranks[original_index] = average_rank
        index = end
    return ranks


def _group_by_tenant(
    observations: list[Observation],
) -> list[list[Observation]]:
    by_tenant: dict[str, list[Observation]] = {}
    for row in observations:
        by_tenant.setdefault(row.tenant_id, []).append(row)
    return [by_tenant[tenant_id] for tenant_id in sorted(by_tenant)]


def _reassign_label_vectors(
    receiving_clusters: list[list[Observation]],
    donor_vectors: list[list[int]],
) -> list[int]:
    labels: list[int] = []
    for receiving, donor in zip(receiving_clusters, donor_vectors):
        if not donor:
            raise AnalyzeGateOutcomeError("permutation_donor_vector_empty")
        labels.extend(
            donor[index % len(donor)] for index in range(len(receiving))
        )
    return labels


def _auc_from_ranks_and_labels(
    ranks: list[float],
    labels: list[int],
) -> Optional[float]:
    positive_count = sum(labels)
    negative_count = len(labels) - positive_count
    if positive_count == 0 or negative_count == 0:
        return None
    positive_rank_sum = sum(
        rank for rank, label in zip(ranks, labels) if label == FAILURE
    )
    wins = positive_rank_sum - positive_count * (positive_count + 1) / 2.0
    return wins / (positive_count * negative_count)


def cluster_label_permutation_test(
    observations: list[Observation],
    signal: str,
    *,
    iterations: int,
    seed: int,
) -> PermutationResult:
    """Permute whole tenant label vectors and test |AUC - 0.5|."""

    clusters = _group_by_tenant(observations)
    scores = [row.values[signal] for cluster in clusters for row in cluster]
    labels = [row.label for cluster in clusters for row in cluster]
    ranks = rank_values(scores)
    observed_auc = _auc_from_ranks_and_labels(ranks, labels)
    if observed_auc is None:
        raise AnalyzeGateOutcomeError("permutation_observed_auc_undefined")
    observed_statistic = abs(observed_auc - 0.5)
    label_vectors = [[row.label for row in cluster] for cluster in clusters]

    rng = random.Random(seed)
    extreme_count = 0
    valid_count = 0
    undefined_count = 0
    for _ in range(iterations):
        donor_vectors = list(label_vectors)
        rng.shuffle(donor_vectors)
        permuted_labels = _reassign_label_vectors(clusters, donor_vectors)
        permuted_auc = _auc_from_ranks_and_labels(ranks, permuted_labels)
        if permuted_auc is None:
            undefined_count += 1
            continue
        valid_count += 1
        if abs(permuted_auc - 0.5) >= observed_statistic:
            extreme_count += 1

    if valid_count == 0:
        raise AnalyzeGateOutcomeError("permutation_produced_no_estimates")
    return PermutationResult(
        p_value=(extreme_count + 1) / (valid_count + 1),
        extreme_count=extreme_count,
        valid_count=valid_count,
        undefined_count=undefined_count,
        iterations=iterations,
    )


def simulate_power_at_alternative(
    observations: list[Observation],
    signal: str,
    *,
    alternative_auc: float,
    simulations: int,
    permutations: int,
    alpha: float,
    seed: int,
) -> PowerResult:
    """Estimate power after whole-cluster resampling at a specified AUC."""

    source_clusters = _group_by_tenant(observations)
    shift = math.sqrt(2.0) * NormalDist().inv_cdf(alternative_auc)
    rng = random.Random(seed)
    rejection_count = 0
    valid_count = 0
    undefined_count = 0

    for simulation in range(simulations):
        simulated: list[Observation] = []
        for slot in range(len(source_clusters)):
            source = rng.choice(source_clusters)
            for row_index, row in enumerate(source):
                score = rng.gauss(shift if row.label == FAILURE else 0.0, 1.0)
                simulated.append(
                    Observation(
                        tenant_id=f"simulation-{simulation}-cluster-{slot}",
                        submission_id=f"row-{row_index}",
                        label=row.label,
                        decided_at=0.0,
                        values={signal: score},
                    )
                )
        positive, negative = _scores(simulated, signal)
        if not positive or not negative:
            undefined_count += 1
            continue
        permutation = cluster_label_permutation_test(
            simulated,
            signal,
            iterations=permutations,
            seed=rng.randrange(0, 2**63),
        )
        valid_count += 1
        if permutation.p_value < alpha:
            rejection_count += 1

    if valid_count == 0:
        raise AnalyzeGateOutcomeError("power_simulation_produced_no_estimates")
    return PowerResult(
        achieved_power=rejection_count / valid_count,
        rejection_count=rejection_count,
        valid_count=valid_count,
        undefined_count=undefined_count,
        iterations=simulations,
    )


def _parse_timestamp(value: str, row_number: int) -> float:
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError as exc:
        raise AnalyzeGateOutcomeError(
            f"invalid_decided_at row={row_number}"
        ) from exc
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.timestamp()


def _parse_number(value: Optional[str], column: str, row_number: int) -> float:
    if value is None or not value.strip():
        raise AnalyzeGateOutcomeError(
            f"missing_numeric_value row={row_number} column={column}"
        )
    try:
        number = float(value)
    except ValueError as exc:
        raise AnalyzeGateOutcomeError(
            f"invalid_numeric_value row={row_number} column={column}"
        ) from exc
    if not math.isfinite(number):
        raise AnalyzeGateOutcomeError(
            f"non_finite_numeric_value row={row_number} column={column}"
        )
    return number


def _parse_label(value: Optional[str], label_name: str) -> Optional[int]:
    normalized = "" if value is None else value.strip().lower()
    if label_name == "task_success":
        if normalized == "failure":
            return FAILURE
        if normalized == "success":
            return SUCCESS
        if normalized in ("partial", "unknown"):
            return None
        raise AnalyzeGateOutcomeError(
            "invalid_task_success "
            "expected=success,partial,failure,unknown"
        )

    if normalized == "true":
        return FAILURE
    if normalized == "false":
        return SUCCESS
    if normalized in ("partial", "unknown"):
        return None
    raise AnalyzeGateOutcomeError(
        "invalid_human_correction expected=true,false,partial,unknown"
    )


def load_observations(
    path: Path,
    label_name: str,
) -> LoadedObservations:
    observations: list[Observation] = []
    excluded = {"partial": 0, "unknown": 0}
    seen: set[tuple[str, str]] = set()

    try:
        handle = path.open("r", encoding="utf-8", newline="")
    except OSError as exc:
        raise AnalyzeGateOutcomeError(
            f"input_open_failed detail={type(exc).__name__}"
        ) from exc

    with handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise AnalyzeGateOutcomeError("input_header_missing")
        missing = [
            name for name in REQUIRED_COLUMNS if name not in reader.fieldnames
        ]
        if missing:
            raise AnalyzeGateOutcomeError(
                f"input_columns_missing columns={','.join(missing)}"
            )
        unknown = [
            name
            for name in reader.fieldnames
            if name not in REQUIRED_COLUMNS and name not in OPTIONAL_COLUMNS
        ]
        if unknown:
            raise AnalyzeGateOutcomeError(
                f"input_columns_unknown count={len(unknown)}"
            )
        if label_name not in reader.fieldnames:
            raise AnalyzeGateOutcomeError(
                f"label_column_missing column={label_name}"
            )

        has_length = "length" in reader.fieldnames
        has_credit_quality = "credit_quality_micros" in reader.fieldnames
        credit_quality_nulls = 0
        total_rows = 0
        for row_number, row in enumerate(reader, start=2):
            total_rows += 1
            tenant_id = (row.get("tenant_id") or "").strip()
            submission_id = (row.get("submission_id") or "").strip()
            if not tenant_id or not submission_id:
                raise AnalyzeGateOutcomeError(
                    f"identity_missing row={row_number}"
                )
            identity = (tenant_id, submission_id)
            if identity in seen:
                raise AnalyzeGateOutcomeError(
                    f"duplicate_submission row={row_number}"
                )
            seen.add(identity)

            raw_label = row.get(label_name)
            label = _parse_label(raw_label, label_name)
            if label is None:
                normalized = (
                    "" if raw_label is None else raw_label.strip().lower()
                )
                if normalized == "partial":
                    excluded["partial"] += 1
                else:
                    excluded["unknown"] += 1
                continue

            values = {
                signal: _parse_number(row.get(column), column, row_number)
                for signal, column in SIGNALS.items()
            }
            if has_length:
                values["length"] = _parse_number(
                    row.get("length"), "length", row_number
                )
            if has_credit_quality:
                raw_credit_quality = row.get("credit_quality_micros")
                if raw_credit_quality is None or not raw_credit_quality.strip():
                    credit_quality_nulls += 1
                else:
                    values["credit_quality"] = _parse_number(
                        raw_credit_quality,
                        "credit_quality_micros",
                        row_number,
                    )
            observations.append(
                Observation(
                    tenant_id=tenant_id,
                    submission_id=submission_id,
                    label=label,
                    decided_at=_parse_timestamp(
                        row.get("decided_at") or "", row_number
                    ),
                    values=values,
                )
            )

    return LoadedObservations(
        observations=observations,
        excluded=excluded,
        has_length=has_length,
        has_credit_quality=has_credit_quality,
        credit_quality_nulls=credit_quality_nulls,
        total_rows=total_rows,
    )


def _interval_excludes_chance(interval: tuple[float, float]) -> bool:
    return interval[1] < 0.5 or interval[0] > 0.5


def _format_interval(interval: tuple[float, float]) -> str:
    return f"[{interval[0]:.4f},{interval[1]:.4f}]"


def _validate_bootstrap_result(
    result: BootstrapResult,
    *,
    signal: str,
    mode: str,
    minimum_valid_fraction: float,
) -> float:
    if result.valid_fraction < minimum_valid_fraction:
        raise AnalyzeGateOutcomeError(
            "bootstrap_valid_fraction_below_min "
            f"signal={signal} mode={mode} valid={result.valid_count} "
            f"undefined={result.undefined_count} "
            f"fraction={result.valid_fraction:.4f} "
            f"required={minimum_valid_fraction:.4f}"
        )
    if result.standard_error is None:
        raise AnalyzeGateOutcomeError(
            "bootstrap_valid_estimates_below_two "
            f"signal={signal} mode={mode} valid={result.valid_count}"
        )
    return result.standard_error


def analyze_signal(
    observations: list[Observation],
    signal: str,
    role: str,
    *,
    null_excluded: int,
    iterations: int,
    permutations: int,
    seed: int,
    alpha: float,
    alternative_auc: Optional[float],
    minimum_valid_fraction: float,
) -> SignalAnalysis:
    positive, negative = _scores(observations, signal)
    auc = discrimination_auc(positive, negative)
    naive = bootstrap_auc_interval(
        observations,
        signal,
        iterations=iterations,
        seed=seed + 10_000,
        clustered=False,
    )
    clustered = bootstrap_auc_interval(
        observations,
        signal,
        iterations=iterations,
        seed=seed,
        clustered=True,
    )
    _validate_bootstrap_result(
        naive,
        signal=signal,
        mode="naive",
        minimum_valid_fraction=minimum_valid_fraction,
    )
    _validate_bootstrap_result(
        clustered,
        signal=signal,
        mode="clustered",
        minimum_valid_fraction=minimum_valid_fraction,
    )
    permutation = cluster_label_permutation_test(
        observations,
        signal,
        iterations=permutations,
        seed=seed + 20_000,
    )
    power = None
    if alternative_auc is not None:
        power = simulate_power_at_alternative(
            observations,
            signal,
            alternative_auc=alternative_auc,
            simulations=iterations,
            permutations=permutations,
            alpha=alpha,
            seed=seed + 30_000,
        )
    status = "ASSOCIATION" if permutation.p_value < alpha else "INCONCLUSIVE"
    return SignalAnalysis(
        name=signal,
        role=role,
        included=len(observations),
        null_excluded=null_excluded,
        auc=auc,
        naive=naive,
        clustered=clustered,
        permutation=permutation,
        power=power,
        status=status,
    )


def _parser() -> OperatorArgumentParser:
    parser = OperatorArgumentParser(
        description="Analyze gate signals against trace outcome labels."
    )
    parser.add_argument("--input", required=True, help="Outcome-join CSV path.")
    parser.add_argument(
        "--label",
        choices=("task_success", "human_correction"),
        default="task_success",
        help="Outcome label (default: task_success).",
    )
    parser.add_argument(
        "--min-per-class",
        type=int,
        default=125,
        help="Minimum labeled rows in each class (default: 125).",
    )
    parser.add_argument(
        "--min-clusters",
        type=int,
        default=10,
        help="Minimum independent tenant clusters (default: 10).",
    )
    parser.add_argument(
        "--bootstrap",
        type=int,
        default=400,
        help="Bootstrap iterations (default: 400).",
    )
    parser.add_argument(
        "--permutations",
        type=int,
        default=400,
        help="Cluster-level label permutations (default: 400).",
    )
    parser.add_argument(
        "--alpha",
        type=float,
        default=0.05,
        help="Permutation-test significance threshold (default: 0.05).",
    )
    parser.add_argument(
        "--alternative-auc",
        type=float,
        help=(
            "Prespecified AUC for optional cluster-resampled power simulation; "
            "omit to make no power claim."
        ),
    )
    parser.add_argument(
        "--min-bootstrap-valid-fraction",
        type=float,
        default=0.80,
        help=(
            "Minimum fraction of bootstrap replicates containing both outcome "
            "classes; lower fractions fail closed (default: 0.80)."
        ),
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=12345,
        help="Bootstrap seed (default: 12345).",
    )
    return parser


def main(argv: Optional[list[str]] = None) -> int:
    try:
        args = _parser().parse_args(argv)
        if args.min_per_class <= 0:
            raise AnalyzeGateOutcomeError("min_per_class_must_be_positive")
        if args.min_clusters <= 0:
            raise AnalyzeGateOutcomeError("min_clusters_must_be_positive")
        if args.bootstrap < 2:
            raise AnalyzeGateOutcomeError("bootstrap_must_be_at_least_two")
        if args.permutations <= 0:
            raise AnalyzeGateOutcomeError("permutations_must_be_positive")
        if not 0.0 < args.alpha < 1.0:
            raise AnalyzeGateOutcomeError("alpha_out_of_range")
        if args.alternative_auc is not None and (
            not 0.0 < args.alternative_auc < 1.0
            or args.alternative_auc == 0.5
        ):
            raise AnalyzeGateOutcomeError("alternative_auc_must_differ_from_chance")
        if not 0.0 < args.min_bootstrap_valid_fraction <= 1.0:
            raise AnalyzeGateOutcomeError(
                "min_bootstrap_valid_fraction_out_of_range"
            )

        input_path = Path(args.input)
        if not input_path.is_file():
            raise AnalyzeGateOutcomeError("input_not_found")
        loaded = load_observations(input_path, args.label)
        observations = loaded.observations
        if not observations:
            raise AnalyzeGateOutcomeError("no_labeled_rows")

        labels_by_tenant: dict[str, list[int]] = {}
        for row in observations:
            labels_by_tenant.setdefault(row.tenant_id, []).append(row.label)
        tenant_count = len(labels_by_tenant)
        if tenant_count < args.min_clusters:
            print(
                "AnalyzeGateOutcomeFailure: INSUFFICIENT_CLUSTERS "
                f"count={tenant_count} required={args.min_clusters}",
                file=sys.stderr,
            )
            return 2

        signal_specs: list[
            tuple[str, str, list[Observation], int, int]
        ] = []
        if loaded.has_credit_quality:
            primary_observations = [
                row for row in observations if "credit_quality" in row.values
            ]
            signal_specs.append(
                (
                    "credit_quality",
                    "primary_combined_graded_credit_score",
                    primary_observations,
                    loaded.credit_quality_nulls,
                    3,
                )
            )
        for offset, signal in enumerate(SIGNALS):
            signal_specs.append(
                (signal, "component", observations, 0, offset)
            )

        for signal, _, signal_rows, _, _ in signal_specs:
            signal_tenants = len({row.tenant_id for row in signal_rows})
            if signal_tenants < args.min_clusters:
                print(
                    "AnalyzeGateOutcomeFailure: "
                    "INSUFFICIENT_SIGNAL_CLUSTERS "
                    f"signal={signal} count={signal_tenants} "
                    f"required={args.min_clusters}",
                    file=sys.stderr,
                )
                return 2
            positive, negative = _scores(signal_rows, signal)
            if (
                len(positive) < args.min_per_class
                or len(negative) < args.min_per_class
            ):
                print(
                    "AnalyzeGateOutcomeFailure: "
                    "class_count_below_min "
                    f"signal={signal} failure={len(positive)} "
                    f"success={len(negative)} "
                    f"min_per_class={args.min_per_class} "
                    f"required_raw_per_class={args.min_per_class}",
                    file=sys.stderr,
                )
                return 2

        analyses = [
            analyze_signal(
                signal_rows,
                signal,
                role,
                null_excluded=null_excluded,
                iterations=args.bootstrap,
                permutations=args.permutations,
                seed=args.seed + offset,
                alpha=args.alpha,
                alternative_auc=args.alternative_auc,
                minimum_valid_fraction=args.min_bootstrap_valid_fraction,
            )
            for signal, role, signal_rows, null_excluded, offset in signal_specs
        ]

        icc_signed = intraclass_correlation(labels_by_tenant)
        icc_conservative_floor = max(0.0, icc_signed)
        size_weighted_mean = size_weighted_mean_cluster_size(
            labels_by_tenant
        )
        effect = design_effect(icc_conservative_floor, size_weighted_mean)

        failure_count = sum(row.label == FAILURE for row in observations)
        success_count = sum(row.label == SUCCESS for row in observations)

        excluded_total = (
            loaded.excluded["partial"] + loaded.excluded["unknown"]
        )
        print(
            "# AnalyzeGateOutcome "
            f"rows={loaded.total_rows} included={len(observations)} "
            f"excluded={excluded_total} tenants={tenant_count} "
            f"label={args.label} bootstrap={args.bootstrap} "
            f"permutations={args.permutations} alpha={args.alpha:.4f} "
            f"seed={args.seed}"
        )
        print(
            "INDEPENDENCE unit=tenant "
            "rows_do_not_replace_independent_tenants "
            f"min_clusters={args.min_clusters}"
        )
        print(
            "LABEL_COUNTS "
            f"failure={failure_count} success={success_count} "
            f"partial_excluded={loaded.excluded['partial']} "
            f"unknown_excluded={loaded.excluded['unknown']}"
        )
        print(
            "CLUSTERING "
            f"icc_signed={icc_signed:.4f} "
            f"icc_conservative_floor={icc_conservative_floor:.4f} "
            f"mA={size_weighted_mean:.4f} "
            f"design_effect={effect:.4f} "
            "diagnostic=label_clustering_only"
        )

        if args.alternative_auc is None:
            print(
                "POWER basis=not_claimed "
                "minimum_detectable_effect_requires_prespecified_alternative "
                "flag=--alternative-auc"
            )

        if not loaded.has_credit_quality:
            print(
                "PRIMARY_SIGNAL credit_quality status=NOT_ANALYZED "
                "reason=input_column_absent "
                "component_aucs_do_not_establish_grading_score_predicts_outcome"
            )

        for analysis in analyses:
            print(
                f"BOOTSTRAP signal={analysis.name} "
                f"naive_valid={analysis.naive.valid_count} "
                f"naive_undefined={analysis.naive.undefined_count} "
                f"clustered_valid={analysis.clustered.valid_count} "
                f"clustered_undefined={analysis.clustered.undefined_count} "
                f"min_valid_fraction={args.min_bootstrap_valid_fraction:.4f}"
            )
            if analysis.power is not None:
                print(
                    "POWER basis=cluster_resampling "
                    f"signal={analysis.name} "
                    f"alternative_auc={args.alternative_auc:.4f} "
                    f"achieved_power={analysis.power.achieved_power:.4f} "
                    f"rejections={analysis.power.rejection_count} "
                    f"valid={analysis.power.valid_count} "
                    f"undefined={analysis.power.undefined_count} "
                    f"simulations={analysis.power.iterations} "
                    f"alpha={args.alpha:.4f}"
                )
            print(
                f"SIGNAL {analysis.name} role={analysis.role} "
                f"included={analysis.included} "
                f"null_excluded={analysis.null_excluded} "
                f"auc={analysis.auc:.4f} "
                f"naive_95={_format_interval(analysis.naive.interval)} "
                "clustered_95="
                f"{_format_interval(analysis.clustered.interval)} "
                f"permutation_p={analysis.permutation.p_value:.6f} "
                f"permutations={analysis.permutation.iterations} "
                f"permutation_valid={analysis.permutation.valid_count} "
                "permutation_undefined="
                f"{analysis.permutation.undefined_count} "
                f"status={analysis.status}"
            )
            if (
                _interval_excludes_chance(analysis.naive.interval)
                and not _interval_excludes_chance(
                    analysis.clustered.interval
                )
            ):
                print(
                    "AnalyzeGateOutcomeWarning: "
                    f"signal={analysis.name} independence_artifact "
                    "naive_95="
                    f"{_format_interval(analysis.naive.interval)} "
                    "clustered_95="
                    f"{_format_interval(analysis.clustered.interval)}",
                    file=sys.stderr,
                )

        covariates: dict[str, float] = {}
        if loaded.has_length:
            positive, negative = _scores(observations, "length")
            covariates["length"] = discrimination_auc(positive, negative)
            print(f"COVARIATE length auc={covariates['length']:.4f}")

        drift_ranks = rank_values([row.decided_at for row in observations])
        drift_positive = [
            rank
            for row, rank in zip(observations, drift_ranks)
            if row.label == FAILURE
        ]
        drift_negative = [
            rank
            for row, rank in zip(observations, drift_ranks)
            if row.label == SUCCESS
        ]
        covariates["decided_at_rank"] = discrimination_auc(
            drift_positive, drift_negative
        )
        print(
            "COVARIATE decided_at_rank "
            f"auc={covariates['decided_at_rank']:.4f}"
        )

        print("# AnalyzeGateOutcomeComplete")
        return 0
    except (AnalyzeGateOutcomeError, csv.Error) as exc:
        print(f"AnalyzeGateOutcomeFailure: {exc}", file=sys.stderr)
        return 2
    except OSError as exc:
        print(
            "AnalyzeGateOutcomeFailure: "
            f"input_read_failed detail={type(exc).__name__}",
            file=sys.stderr,
        )
        return 2


if __name__ == "__main__":
    sys.exit(main())
