#!/usr/bin/env python3
"""Smoke test for `a5a_rarity_preflight.py`.

Runs the pre-flight script against the synthetic 8/8/2 fixture with the
mock-logprobs scorer (so no transformers/torch dependency), and asserts:

  1. The report JSON has the expected top-level shape (candidate_id,
     corpus_sha256, model_sha_or_revision, k_values, per_k_results,
     elapsed_seconds, tokens_per_second, slice_counts,
     perplexity_baseline).
  2. Each requested K has `auc_novel_vs_duplicate` and a
     `rarity_scores_micros` block with the right slice cardinalities.
  3. Re-running with the same seed produces byte-identical AUC values
     and rarity_scores_micros lists (the determinism property).

Designed to work under pytest or as a standalone script.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "a5a_rarity_preflight.py"
FIXTURE = HERE / "fixtures" / "synthetic-corpus.tar.zst"

EXPECTED_K_VALUES = [4, 8, 16, 32]


def _run(report_out: Path, seed: int) -> dict:
    cmd = [
        sys.executable,
        str(SCRIPT),
        "--candidate",
        "synthetic-test",
        "--corpus",
        str(FIXTURE),
        "--k-values",
        ",".join(str(k) for k in EXPECTED_K_VALUES),
        "--report-out",
        str(report_out),
        "--mock-logprobs-seed",
        str(seed),
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise AssertionError(
            f"pre-flight script exited {result.returncode}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
    return json.loads(report_out.read_text())


def test_report_shape() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "report.json"
        report = _run(out, seed=42)

    top_level = {
        "candidate_id",
        "corpus_sha256",
        "model_sha_or_revision",
        "k_values",
        "slice_counts",
        "perplexity_baseline",
        "per_k_results",
        "elapsed_seconds",
        "tokens_per_second",
    }
    missing = top_level - set(report)
    assert not missing, f"missing top-level keys: {missing}"

    assert report["candidate_id"] == "synthetic-test"
    assert report["corpus_sha256"].startswith("sha256:")
    assert report["model_sha_or_revision"].startswith("mock_logprobs:seed=")
    assert report["k_values"] == EXPECTED_K_VALUES

    counts = report["slice_counts"]
    # synthetic-corpus.tar.zst has 8 novel + 8 duplicate + 4 paraphrase pairs.
    assert counts == {"novel": 8, "duplicate": 8, "paraphrase": 4}, counts

    baseline = report["perplexity_baseline"]
    assert "auc_novel_vs_duplicate" in baseline
    assert 0.0 <= baseline["auc_novel_vs_duplicate"] <= 1.0

    for k in EXPECTED_K_VALUES:
        block = report["per_k_results"][str(k)]
        assert "auc_novel_vs_duplicate" in block
        auc = block["auc_novel_vs_duplicate"]
        assert 0.0 <= auc <= 1.0, f"K={k} AUC out of range: {auc}"
        scores = block["rarity_scores_micros"]
        assert len(scores["novel"]) == counts["novel"]
        assert len(scores["duplicate"]) == counts["duplicate"]
        assert len(scores["paraphrase"]) == counts["paraphrase"]
        for slice_name, vals in scores.items():
            for v in vals:
                assert isinstance(v, int), f"{slice_name} score not int: {v!r}"
                assert v >= 0, f"{slice_name} score negative: {v}"


def test_determinism() -> None:
    """Two runs with the same seed must produce identical AUC values and
    identical rarity_scores_micros lists. Elapsed-time and throughput are
    permitted to differ (wall-clock derived) and explicitly excluded from
    the comparison.
    """
    with tempfile.TemporaryDirectory() as tmp:
        out_a = Path(tmp) / "a.json"
        out_b = Path(tmp) / "b.json"
        report_a = _run(out_a, seed=7)
        report_b = _run(out_b, seed=7)

    # Drop wall-clock fields before comparing.
    for r in (report_a, report_b):
        r.pop("elapsed_seconds", None)
        r.pop("tokens_per_second", None)
    assert report_a == report_b, (
        "non-deterministic output:\n"
        f"a: {json.dumps(report_a, sort_keys=True)[:300]}\n"
        f"b: {json.dumps(report_b, sort_keys=True)[:300]}"
    )


def test_different_seeds_differ() -> None:
    """Sanity: different seeds produce different AUC trajectories (otherwise
    the mock is a constant and the determinism test is vacuous)."""
    with tempfile.TemporaryDirectory() as tmp:
        out_a = Path(tmp) / "a.json"
        out_b = Path(tmp) / "b.json"
        report_a = _run(out_a, seed=1)
        report_b = _run(out_b, seed=2)

    # At least one K must produce a different rarity_scores_micros list
    # under the alternate seed.
    for k in EXPECTED_K_VALUES:
        a = report_a["per_k_results"][str(k)]["rarity_scores_micros"]
        b = report_b["per_k_results"][str(k)]["rarity_scores_micros"]
        if a != b:
            return
    raise AssertionError(
        "rarity_scores_micros identical across two distinct seeds; mock too weak"
    )


def main() -> int:
    test_report_shape()
    print("test_report_shape: ok")
    test_determinism()
    print("test_determinism: ok")
    test_different_seeds_differ()
    print("test_different_seeds_differ: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
