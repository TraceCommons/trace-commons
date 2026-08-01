#!/usr/bin/env python3
"""
a27_calibrate_perplexity_floor.py — compute the recommended
`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` value from a bake-off
report JSON containing per_trace_scores.

Per docs/operator/a27-perplexity-floor-calibration.md and the A2.7
spec, the recommended floor is:

    floor_raw       = sqrt(youden_j_optimum * p10_novel)
    floor_proposed  = floor_raw * 0.5             # 0.5x headroom margin
    floor_micros    = round(floor_proposed * 1e6)

where:
    - youden_j_optimum = perplexity threshold that maximizes
      Youden's J (TPR - FPR) on novel-vs-duplicate classification
    - p10_novel       = 10th percentile of novel-slice perplexity
    - calibration candidate = worst-of-passing in the report (AUC>0.5,
      passed_determinism_gate, and zero score-failure rate on novel;
      decision-rule v3 also requires a report winner, baseline dominance,
      and complete novel/duplicate/paraphrase support)

Stdlib-only. No new dependencies.

Usage:
    python3 a27_calibrate_perplexity_floor.py path/to/report.json

Optional args:
    --headroom 0.5    Headroom multiplier (default 0.5)
    --candidate ID    Force a specific candidate by id instead of
                      auto-picking worst-of-passing
"""

import argparse
import json
import math
import statistics
import sys


SUPPORTED_DECISION_RULE_VERSIONS = frozenset((1, 2, 3))


def auc_from_scores(novel, duplicate):
    """Mann-Whitney U style AUC: P(score(novel) > score(duplicate))."""
    novel = [n for n in novel if n is not None]
    duplicate = [d for d in duplicate if d is not None]
    if not novel or not duplicate:
        return float("nan")
    wins = 0.0
    for n in novel:
        for d in duplicate:
            if n > d:
                wins += 1.0
            elif n == d:
                wins += 0.5
    return wins / (len(novel) * len(duplicate))


def youden_j_optimum(novel, duplicate):
    """Find the threshold maximizing Youden's J (TPR - FPR).

    Convention: NOVEL is "positive" and we want to flag novel
    submissions ABOVE the threshold (high perplexity = novel).
    """
    novel = [n for n in novel if n is not None]
    duplicate = [d for d in duplicate if d is not None]
    n_n = len(novel)
    n_d = len(duplicate)
    if n_n == 0 or n_d == 0:
        return float("nan")

    thresholds = sorted(set(novel + duplicate))
    best_j = -1.0
    best_t = thresholds[0]
    for t in thresholds:
        tp = sum(1 for v in novel if v > t)
        fp = sum(1 for v in duplicate if v > t)
        tpr = tp / n_n
        fpr = fp / n_d
        j = tpr - fpr
        if j > best_j:
            best_j = j
            best_t = t
    return best_t


def percentile(values, p):
    """Stdlib nearest-rank percentile (no interpolation)."""
    cleaned = sorted(v for v in values if v is not None)
    if not cleaned:
        return float("nan")
    k = max(0, min(len(cleaned) - 1, int(math.ceil(p / 100.0 * len(cleaned))) - 1))
    return cleaned[k]


def decision_rule_version(report):
    """Return a supported rule version, rejecting bools and other JSON types."""
    version = report.get("decision_rule_version")
    if type(version) is not int or version not in SUPPORTED_DECISION_RULE_VERSIONS:
        return None
    return version


def pick_calibration_candidate(report):
    """Worst-of-passing: lowest AUC among candidates that:
       - AUC > 0.5
       - passed_determinism_gate = true
       - have non-null per_trace_scores
       - have no null entries in the novel slice (zero score failures)
       - under decision-rule v3, the report has a winner and the candidate
         passed baseline dominance with no dropped decision-metric rows
    """
    version = decision_rule_version(report)
    if version is None:
        return None
    if version == 3 and report.get("winner_id") is None:
        return None

    eligible = []
    for c in report["candidates"]:
        if c.get("discrimination_auc", 0) <= 0.5:
            continue
        if not c.get("passed_determinism_gate"):
            continue
        scores = c.get("per_trace_scores")
        if not scores:
            continue
        novel = scores.get("novel") or []
        if not novel or any(v is None for v in novel):
            continue
        if version == 3:
            if not c.get("passed_baseline_dominance"):
                continue
            if c.get("dropped_novel_rows") != 0:
                continue
            if c.get("dropped_duplicate_rows") != 0:
                continue
            if c.get("dropped_paraphrase_rows") != 0:
                continue
        eligible.append(c)
    if not eligible:
        return None
    eligible.sort(key=lambda c: c["discrimination_auc"])
    return eligible[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("report", help="path to bake-off report.json")
    ap.add_argument("--headroom", type=float, default=0.5,
                    help="headroom multiplier (default 0.5)")
    ap.add_argument("--candidate", default=None,
                    help="force a specific candidate by id")
    args = ap.parse_args()

    with open(args.report) as f:
        report = json.load(f)

    if decision_rule_version(report) is None:
        print("error: invalid or unsupported decision_rule_version", file=sys.stderr)
        sys.exit(2)

    if args.candidate:
        target = next((c for c in report["candidates"]
                      if c["id"] == args.candidate), None)
        if not target:
            print(f"error: candidate '{args.candidate}' not in report",
                  file=sys.stderr)
            sys.exit(2)
    else:
        target = pick_calibration_candidate(report)
        if not target:
            print("error: no eligible calibration candidate "
                  "(need AUC>0.5, passed_det_gate, "
                  "per_trace_scores, no nulls in novel slice)",
                  file=sys.stderr)
            sys.exit(2)

    scores = target["per_trace_scores"]
    novel = scores["novel"]
    duplicate = scores["duplicate"]

    auc_reported = target["discrimination_auc"]
    auc_recomputed = auc_from_scores(novel, duplicate)
    j_optimum = youden_j_optimum(novel, duplicate)
    p10_novel = percentile(novel, 10)
    median_novel = percentile(novel, 50)

    if j_optimum is None or math.isnan(j_optimum):
        print("error: Youden's J optimum could not be computed",
              file=sys.stderr)
        sys.exit(2)
    if p10_novel is None or math.isnan(p10_novel):
        print("error: p10 novel could not be computed", file=sys.stderr)
        sys.exit(2)

    if j_optimum <= 0 or p10_novel <= 0:
        print(f"error: non-positive anchor (j={j_optimum}, "
              f"p10={p10_novel}); geometric mean undefined",
              file=sys.stderr)
        sys.exit(2)

    floor_raw = math.sqrt(j_optimum * p10_novel)
    floor_proposed = floor_raw * args.headroom
    if floor_proposed > median_novel:
        print(f"warning: floor_proposed ({floor_proposed:.4f}) > "
              f"median_novel ({median_novel:.4f}); floor would "
              f"reject more than half of novel; consider tightening "
              f"headroom", file=sys.stderr)
    floor_micros = max(0, round(floor_proposed * 1_000_000))

    out = {
        "calibration_candidate": target["id"],
        "discrimination_auc_reported": auc_reported,
        "discrimination_auc_recomputed": auc_recomputed,
        "youden_j_optimum_perplexity": j_optimum,
        "p10_novel_perplexity": p10_novel,
        "median_novel_perplexity": median_novel,
        "headroom_multiplier": args.headroom,
        "floor_raw_perplexity": floor_raw,
        "floor_proposed_perplexity": floor_proposed,
        "floor_proposed_micros": floor_micros,
        "env_var_line": f"TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS={floor_micros}",
    }
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
