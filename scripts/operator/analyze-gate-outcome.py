#!/usr/bin/env python3
"""Measure whether gate signals discriminate failed from successful traces.

Expected CSV schema (header required, one row per scored submission):

    tenant_id,submission_id,decided_at,perplexity_micros,tail_fraction_micros,novelty_score_micros,task_success

`task_success` must be `success`, `partial`, `failure`, or `unknown`.
Partial and unknown rows are counted and excluded from both AUC arms. An
optional final `length` column enables the trace-length covariate. For
`--label=human_correction`, append a `human_correction` column containing
`true`, `false`, `partial`, or `unknown`; true is the failure arm.

At least 10 independent tenant clusters are required by default because a
cluster bootstrap with fewer resampling units cannot support a reliable
estimate of between-cluster uncertainty. Override with `--min-clusters`
only when a different threshold is justified before examining outcomes.

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
      SELECT g.tenant_id, g.submission_id, g.decided_at,
             g.perplexity_micros, g.tail_fraction_micros,
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
from statistics import NormalDist
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
OPTIONAL_COLUMNS = ("human_correction", "length")
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
    if denominator <= 0.0:
        return 0.0
    return max(0.0, (between_ms - within_ms) / denominator)


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
    return max(1.0, 1.0 + (size_weighted_mean - 1.0) * icc)


def minimum_detectable_auc(
    positive_n: float,
    negative_n: float,
    *,
    alpha: float = 0.05,
    power: float = 0.80,
) -> float:
    """Invert the Hanley–McNeil AUC variance for a two-sided 80% test."""

    if positive_n <= 1.0 or negative_n <= 1.0:
        return 1.0

    normal = NormalDist()
    target_z = normal.inv_cdf(1.0 - alpha / 2.0) + normal.inv_cdf(power)

    def standardized_effect(auc: float) -> float:
        q1 = auc / (2.0 - auc)
        q2 = 2.0 * auc * auc / (1.0 + auc)
        variance = (
            auc * (1.0 - auc)
            + (positive_n - 1.0) * (q1 - auc * auc)
            + (negative_n - 1.0) * (q2 - auc * auc)
        ) / (positive_n * negative_n)
        if variance <= 0.0:
            return math.inf
        return (auc - 0.5) / math.sqrt(variance)

    low = 0.5
    high = 1.0 - 1e-12
    for _ in range(80):
        midpoint = (low + high) / 2.0
        if standardized_effect(midpoint) >= target_z:
            high = midpoint
        else:
            low = midpoint
    return high


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
) -> tuple[float, float]:
    """Bootstrap rows or whole tenant clusters and return a 95% interval."""

    rng = random.Random(seed)
    estimates: list[float] = []

    if clustered:
        by_tenant: dict[str, list[Observation]] = {}
        for row in observations:
            by_tenant.setdefault(row.tenant_id, []).append(row)
        tenant_ids = sorted(by_tenant)
        if not tenant_ids:
            return 0.5, 0.5
        for _ in range(iterations):
            sampled: list[Observation] = []
            for _ in tenant_ids:
                sampled.extend(by_tenant[rng.choice(tenant_ids)])
            positive, negative = _scores(sampled, signal)
            estimates.append(discrimination_auc(positive, negative))
    else:
        if not observations:
            return 0.5, 0.5
        for _ in range(iterations):
            sampled = [rng.choice(observations) for _ in observations]
            positive, negative = _scores(sampled, signal)
            estimates.append(discrimination_auc(positive, negative))

    return percentile(estimates, 0.025), percentile(estimates, 0.975)


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
) -> tuple[list[Observation], dict[str, int], bool, int]:
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
                f"input_columns_unknown columns={','.join(unknown)}"
            )
        if label_name not in reader.fieldnames:
            raise AnalyzeGateOutcomeError(
                f"label_column_missing column={label_name}"
            )

        has_length = "length" in reader.fieldnames
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

    return observations, excluded, has_length, total_rows


def _interval_excludes_chance(interval: tuple[float, float]) -> bool:
    return interval[1] < 0.5 or interval[0] > 0.5


def _format_interval(interval: tuple[float, float]) -> str:
    return f"[{interval[0]:.4f},{interval[1]:.4f}]"


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
        help="Minimum effective observations in each class (default: 125).",
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
        if args.bootstrap <= 0:
            raise AnalyzeGateOutcomeError("bootstrap_must_be_positive")

        input_path = Path(args.input)
        if not input_path.is_file():
            raise AnalyzeGateOutcomeError("input_not_found")
        observations, excluded, has_length, total_rows = load_observations(
            input_path, args.label
        )
        if not observations:
            raise AnalyzeGateOutcomeError("no_labeled_rows")

        labels_by_tenant: dict[str, list[int]] = {}
        for row in observations:
            labels_by_tenant.setdefault(row.tenant_id, []).append(row.label)
        tenant_count = len(labels_by_tenant)
        if tenant_count < args.min_clusters:
            print(
                "INSUFFICIENT_CLUSTERS "
                f"count={tenant_count} required={args.min_clusters}",
                file=sys.stderr,
            )
            return 2

        icc = intraclass_correlation(labels_by_tenant)
        size_weighted_mean = size_weighted_mean_cluster_size(
            labels_by_tenant
        )
        effect = design_effect(icc, size_weighted_mean)

        failure_count = sum(row.label == FAILURE for row in observations)
        success_count = sum(row.label == SUCCESS for row in observations)
        effective_failure = failure_count / effect
        effective_success = success_count / effect
        effective_total = len(observations) / effect
        mde = minimum_detectable_auc(effective_failure, effective_success)
        undetectable_band = (1.0 - mde, mde)

        excluded_total = excluded["partial"] + excluded["unknown"]
        print(
            "# AnalyzeGateOutcome "
            f"rows={total_rows} included={len(observations)} "
            f"excluded={excluded_total} tenants={tenant_count} "
            f"label={args.label} bootstrap={args.bootstrap} seed={args.seed}"
        )
        print(
            "LABEL_COUNTS "
            f"failure={failure_count} success={success_count} "
            f"partial_excluded={excluded['partial']} "
            f"unknown_excluded={excluded['unknown']}"
        )
        print(
            "CLUSTERING "
            f"icc={icc:.4f} mA={size_weighted_mean:.4f} "
            f"design_effect={effect:.4f} "
            f"effective_n={effective_total:.2f} "
            f"effective_failure={effective_failure:.2f} "
            f"effective_success={effective_success:.2f}"
        )
        print(
            "POWER "
            f"minimum_detectable_auc={mde:.4f} "
            f"undetectable_band={_format_interval(undetectable_band)} "
            "power=0.80 alpha=0.05"
        )

        for offset, signal in enumerate(SIGNALS):
            positive, negative = _scores(observations, signal)
            auc = discrimination_auc(positive, negative)
            naive = bootstrap_auc_interval(
                observations,
                signal,
                iterations=args.bootstrap,
                seed=args.seed + 10_000 + offset,
                clustered=False,
            )
            clustered = bootstrap_auc_interval(
                observations,
                signal,
                iterations=args.bootstrap,
                seed=args.seed + offset,
                clustered=True,
            )
            if undetectable_band[0] <= auc <= undetectable_band[1]:
                status = (
                    "UNDERPOWERED "
                    f"band={_format_interval(undetectable_band)}"
                )
            elif _interval_excludes_chance(clustered):
                status = "ASSOCIATION"
            else:
                status = "INCONCLUSIVE"
            print(
                f"SIGNAL {signal} auc={auc:.4f} "
                f"naive_95={_format_interval(naive)} "
                f"clustered_95={_format_interval(clustered)} "
                f"status={status}"
            )
            if (
                _interval_excludes_chance(naive)
                and not _interval_excludes_chance(clustered)
            ):
                print(
                    "AnalyzeGateOutcomeWarning: "
                    f"signal={signal} independence_artifact "
                    f"naive_95={_format_interval(naive)} "
                    f"clustered_95={_format_interval(clustered)}",
                    file=sys.stderr,
                )

        covariates: dict[str, float] = {}
        if has_length:
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

        if (
            effective_failure < args.min_per_class
            or effective_success < args.min_per_class
        ):
            required_raw = math.ceil(args.min_per_class * effect)
            print(
                "AnalyzeGateOutcomeFailure: "
                "effective_class_count_below_min "
                f"effective_failure={effective_failure:.2f} "
                f"effective_success={effective_success:.2f} "
                f"min_per_class={args.min_per_class} "
                f"required_raw_per_class={required_raw}",
                file=sys.stderr,
            )
            return 2

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
