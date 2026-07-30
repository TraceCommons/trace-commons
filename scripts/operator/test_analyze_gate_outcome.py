#!/usr/bin/env python3
"""Tests for `analyze-gate-outcome.py`.

Designed to work either under pytest:

    pytest scripts/operator/test_analyze_gate_outcome.py

or as a standalone script:

    python3 scripts/operator/test_analyze_gate_outcome.py

Exits non-zero on any failed assertion.
"""

from __future__ import annotations

import csv
import importlib.util
import re
import subprocess
import sys
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "analyze-gate-outcome.py"
FIXTURE = HERE / "fixtures" / "gate-outcome" / "sample.csv"


def _load_module():
    spec = importlib.util.spec_from_file_location(
        "analyze_gate_outcome", SCRIPT
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules["analyze_gate_outcome"] = module
    spec.loader.exec_module(module)
    return module


M = _load_module()


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
    )


def _observation(
    tenant: str,
    label: int,
    score: float,
    sequence: int,
) -> object:
    return M.Observation(
        tenant_id=tenant,
        submission_id=f"submission-{sequence}",
        label=label,
        decided_at=float(sequence),
        values={"signal": score},
    )


def _write_csv(path: Path, rows: list[dict[str, object]]) -> None:
    fieldnames = [
        "tenant_id",
        "submission_id",
        "decided_at",
        "perplexity_micros",
        "tail_fraction_micros",
        "novelty_score_micros",
        "task_success",
        "length",
    ]
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def test_auc_matches_hand_computed_ties_and_empty_rule() -> None:
    assert M.discrimination_auc([2.0, 2.0], [1.0, 2.0]) == 0.75
    assert M.discrimination_auc([], [1.0]) == 0.5
    assert M.discrimination_auc([1.0], []) == 0.5


def test_icc_independent_label_is_zero() -> None:
    labels = {f"tenant-{i}": [0, 1, 0, 1] for i in range(8)}
    icc = M.intraclass_correlation(labels)
    size_weighted_mean = M.size_weighted_mean_cluster_size(labels)
    assert abs(icc) < 1e-12, icc
    assert abs(size_weighted_mean - 4.0) < 1e-12, size_weighted_mean


def test_icc_constant_within_tenant_is_near_one() -> None:
    labels = {
        "failure-tenant": [1] * 12,
        "success-tenant": [0] * 12,
    }
    icc = M.intraclass_correlation(labels)
    assert icc > 0.99, icc


def test_size_weighted_mean_drives_extreme_imbalance_design_effect() -> None:
    labels = {"large": [0] * 1000}
    labels.update({f"small-{index}": [0, 1] for index in range(25)})
    sizes = [len(values) for values in labels.values()]
    total = sum(sizes)
    cluster_count = len(sizes)

    arithmetic_mean = total / cluster_count
    independent_m0 = (
        total - sum(size * size for size in sizes) / total
    ) / (cluster_count - 1)
    size_weighted_mean = M.size_weighted_mean_cluster_size(labels)

    assert abs(arithmetic_mean - 40.3846154) < 1e-7, arithmetic_mean
    assert abs(independent_m0 - 3.9009524) < 1e-7, independent_m0
    assert abs(size_weighted_mean - 952.4761905) < 1e-7, (
        size_weighted_mean
    )

    icc = 0.2
    m0_effect = M.design_effect(icc, independent_m0)
    weighted_effect = M.design_effect(icc, size_weighted_mean)
    assert weighted_effect > 100.0 * m0_effect, (
        m0_effect,
        weighted_effect,
    )


def test_unequal_cluster_m0_and_icc_regression() -> None:
    labels = {
        "two": [0, 0],
        "four": [1, 1, 0, 0],
        "six": [1, 1, 1, 1, 1, 1],
    }
    sizes = [len(values) for values in labels.values()]
    total = sum(sizes)
    independent_m0 = (
        total - sum(size * size for size in sizes) / total
    ) / (len(sizes) - 1)
    icc = M.intraclass_correlation(labels)

    assert abs(independent_m0 - 3.6666667) < 1e-7, independent_m0
    assert abs(icc - 0.639344) < 1e-6, icc


def test_minimum_detectable_auc_is_point_six_at_125_per_class() -> None:
    mde = M.minimum_detectable_auc(125.0, 125.0)
    assert abs(mde - 0.60) < 0.001, mde


def test_clustered_interval_wider_than_naive_interval() -> None:
    observations = []
    sequence = 0
    tenant_scores = [
        ("failure-1", M.FAILURE, 0.90),
        ("failure-2", M.FAILURE, 0.70),
        ("failure-3", M.FAILURE, 0.40),
        ("failure-4", M.FAILURE, 0.20),
        ("success-1", M.SUCCESS, 0.80),
        ("success-2", M.SUCCESS, 0.60),
        ("success-3", M.SUCCESS, 0.30),
        ("success-4", M.SUCCESS, 0.10),
    ]
    for tenant, label, score in tenant_scores:
        for _ in range(20):
            observations.append(_observation(tenant, label, score, sequence))
            sequence += 1

    naive = M.bootstrap_auc_interval(
        observations,
        "signal",
        iterations=800,
        seed=431,
        clustered=False,
    )
    clustered = M.bootstrap_auc_interval(
        observations,
        "signal",
        iterations=800,
        seed=431,
        clustered=True,
    )
    naive_width = naive[1] - naive[0]
    clustered_width = clustered[1] - clustered[0]
    assert clustered_width > naive_width, (naive, clustered)


def test_script_flags_naive_only_independence_artifact() -> None:
    tenant_scores = [
        ("failure-1", "failure", 900),
        ("failure-2", "failure", 700),
        ("failure-3", "failure", 400),
        ("failure-4", "failure", 200),
        ("success-1", "success", 800),
        ("success-2", "success", 600),
        ("success-3", "success", 300),
        ("success-4", "success", 100),
        ("failure-5", "failure", 650),
        ("success-5", "success", 350),
    ]
    rows = []
    sequence = 0
    for tenant, label, score in tenant_scores:
        for _ in range(20):
            rows.append(
                {
                    "tenant_id": tenant,
                    "submission_id": f"submission-{sequence}",
                    "decided_at": "2026-07-30T00:00:00Z",
                    "perplexity_micros": score,
                    "tail_fraction_micros": score,
                    "novelty_score_micros": score,
                    "task_success": label,
                    "length": score,
                }
            )
            sequence += 1

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "clustered.csv"
        _write_csv(path, rows)
        result = _run(
            [
                f"--input={path}",
                "--min-per-class=1",
                "--bootstrap=800",
                "--seed=431",
            ]
    )
    assert result.returncode == 0, result.stderr
    assert "signal=perplexity independence_artifact" in result.stderr
    assert "AnalyzeGateOutcomeWarning:" not in result.stdout


def test_script_exits_nonzero_when_effective_classes_are_underpowered() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "underpowered.csv"
        rows = []
        for index in range(10):
            failure = index < 5
            rows.append(
                {
                    "tenant_id": f"tenant-{index}",
                    "submission_id": f"submission-{index}",
                    "decided_at": "2026-07-30T00:00:00Z",
                    "perplexity_micros": 900 if failure else 100,
                    "tail_fraction_micros": 800 if failure else 200,
                    "novelty_score_micros": 700 if failure else 300,
                    "task_success": "failure" if failure else "success",
                    "length": 100 if failure else 20,
                }
            )
        _write_csv(path, rows)
        result = _run([f"--input={path}", "--bootstrap=20"])
    assert result.returncode != 0
    assert "AnalyzeGateOutcomeFailure:" in result.stderr
    assert "required_raw_per_class=125" in result.stderr


def test_partial_and_unknown_rows_are_excluded() -> None:
    rows = []
    for sequence, label in enumerate(
        ("failure", "success", "failure", "success", "partial", "unknown")
    ):
        rows.append(
            {
                "tenant_id": f"tenant-{sequence // 2}",
                "submission_id": f"submission-{sequence}",
                "decided_at": "2026-07-30T00:00:00Z",
                "perplexity_micros": sequence,
                "tail_fraction_micros": sequence,
                "novelty_score_micros": sequence,
                "task_success": label,
                "length": sequence + 1,
            }
        )

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "excluded.csv"
        _write_csv(path, rows)
        result = _run(
            [
                f"--input={path}",
                "--min-per-class=1",
                "--min-clusters=2",
                "--bootstrap=20",
            ]
        )
    assert result.returncode == 0, result.stderr
    assert "rows=6 included=4 excluded=2" in result.stdout
    assert "partial_excluded=1 unknown_excluded=1" in result.stdout


def test_script_rejects_one_tenant_as_insufficient_clusters() -> None:
    rows = []
    for index, label in enumerate(("failure", "success")):
        rows.append(
            {
                "tenant_id": "only-tenant",
                "submission_id": f"submission-{index}",
                "decided_at": "2026-07-30T00:00:00Z",
                "perplexity_micros": 900 if label == "failure" else 100,
                "tail_fraction_micros": 800 if label == "failure" else 200,
                "novelty_score_micros": 700 if label == "failure" else 300,
                "task_success": label,
                "length": 100 if label == "failure" else 20,
            }
        )

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "one-tenant.csv"
        _write_csv(path, rows)
        result = _run(
            [
                f"--input={path}",
                "--min-per-class=1",
                "--bootstrap=20",
            ]
        )
    assert result.returncode != 0
    assert "INSUFFICIENT_CLUSTERS count=1 required=10" in result.stderr
    assert "status=ASSOCIATION" not in result.stdout


def test_mde_uses_effective_counts_when_design_effect_exceeds_one() -> None:
    rows = []
    sequence = 0
    for tenant_index in range(10):
        failure = tenant_index < 5
        label = "failure" if failure else "success"
        score = 900 if failure else 100
        for _ in range(40):
            rows.append(
                {
                    "tenant_id": f"tenant-{tenant_index}",
                    "submission_id": f"submission-{sequence}",
                    "decided_at": "2026-07-30T00:00:00Z",
                    "perplexity_micros": score,
                    "tail_fraction_micros": score,
                    "novelty_score_micros": score,
                    "task_success": label,
                    "length": score,
                }
            )
            sequence += 1

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "effective-mde.csv"
        _write_csv(path, rows)
        result = _run(
            [
                f"--input={path}",
                "--min-per-class=1",
                "--bootstrap=20",
                "--seed=19",
            ]
        )
    assert result.returncode == 0, result.stderr

    effect_match = re.search(r"design_effect=([0-9.]+)", result.stdout)
    mde_match = re.search(r"minimum_detectable_auc=([0-9.]+)", result.stdout)
    assert effect_match is not None, result.stdout
    assert mde_match is not None, result.stdout
    effect = float(effect_match.group(1))
    printed_mde = float(mde_match.group(1))
    effective_mde = M.minimum_detectable_auc(200.0 / effect, 200.0 / effect)
    raw_mde = M.minimum_detectable_auc(200.0, 200.0)

    assert effect > 1.0, effect
    assert abs(printed_mde - effective_mde) < 0.0001, (
        printed_mde,
        effective_mde,
    )
    assert abs(printed_mde - raw_mde) > 0.05, (printed_mde, raw_mde)


def test_label_parser_accepts_only_documented_values() -> None:
    assert M._parse_label("failure", "task_success") == M.FAILURE
    assert M._parse_label("success", "task_success") == M.SUCCESS
    assert M._parse_label("partial", "task_success") is None
    assert M._parse_label("unknown", "task_success") is None
    assert M._parse_label("true", "human_correction") == M.FAILURE
    assert M._parse_label("false", "human_correction") == M.SUCCESS
    assert M._parse_label("partial", "human_correction") is None
    assert M._parse_label("unknown", "human_correction") is None

    rejected = (
        ("", "task_success", "invalid_task_success"),
        ("true", "task_success", "invalid_task_success"),
        ("failure", "human_correction", "invalid_human_correction"),
        ("success", "human_correction", "invalid_human_correction"),
        ("1", "human_correction", "invalid_human_correction"),
        ("yes", "human_correction", "invalid_human_correction"),
    )
    for value, label_name, error_prefix in rejected:
        try:
            M._parse_label(value, label_name)
        except M.AnalyzeGateOutcomeError as exc:
            assert str(exc).startswith(error_prefix), str(exc)
        else:
            raise AssertionError((value, label_name))


def test_fixture_has_enough_clusters_and_real_associations() -> None:
    result = _run(
        [
            f"--input={FIXTURE}",
        ]
    )
    assert result.returncode == 0, result.stderr
    assert "tenants=10" in result.stdout
    assert result.stdout.count("status=ASSOCIATION") == 2, result.stdout
    assert "status=UNDERPOWERED band=" in result.stdout
    assert "# AnalyzeGateOutcomeComplete" in result.stdout
    assert "# AnalyzeGateOutcomeOK" not in result.stdout


def main() -> int:
    failures = 0
    for name, function in list(globals().items()):
        if not name.startswith("test_") or not callable(function):
            continue
        try:
            function()
            print(f"ok   {name}")
        except AssertionError as exc:
            failures += 1
            print(f"FAIL {name}: {exc}")
        except Exception as exc:  # noqa: BLE001
            failures += 1
            print(f"ERROR {name}: {type(exc).__name__}: {exc}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
