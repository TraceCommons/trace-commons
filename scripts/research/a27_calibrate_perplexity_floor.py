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
      decision-rule v3 also requires a valid report winner, recomputed baseline
      dominance, and complete novel/duplicate/paraphrase support)

Stdlib-only. No new dependencies.

Usage:
    python3 a27_calibrate_perplexity_floor.py path/to/report.json

Optional args:
    --headroom 0.5    Headroom multiplier (default 0.5)
    --candidate ID    Select a specific eligible candidate by id instead of
                      auto-picking worst-of-passing
"""

import argparse
import json
import math
import statistics
import struct
import sys


SUPPORTED_DECISION_RULE_VERSIONS = frozenset((1, 2, 3))
U64_MAX = (1 << 64) - 1
BASELINE_COMPARISON_ULPS = 4


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


def is_finite_json_number(value):
    """Accept JSON integers/floats, excluding bool and non-finite extensions."""
    return type(value) in (int, float) and math.isfinite(value)


def ordered_float_bits(value):
    """Map an IEEE-754 double to monotonically ordered unsigned bits."""
    sign_bit = 1 << 63
    bits = struct.unpack(">Q", struct.pack(">d", value))[0]
    return bits | sign_bit if bits & sign_bit == 0 else (~bits & U64_MAX)


def clears_required_auc(candidate_auc, required_auc):
    """Mirror Rust's inclusive four-ULP baseline boundary."""
    if not is_finite_json_number(candidate_auc):
        return False
    if not is_finite_json_number(required_auc):
        return False
    return candidate_auc >= required_auc or (
        candidate_auc < required_auc
        and abs(
            ordered_float_bits(candidate_auc) - ordered_float_bits(required_auc)
        )
        <= BASELINE_COMPARISON_ULPS
    )


def candidate_is_eligible(candidate, version, required_auc=None):
    """Apply the version-appropriate eligibility checks to one candidate."""
    auc = candidate.get("discrimination_auc")
    if not is_finite_json_number(auc) or auc <= 0.5:
        return False

    passed_determinism_gate = candidate.get("passed_determinism_gate")
    if type(passed_determinism_gate) is not bool or not passed_determinism_gate:
        return False

    scores = candidate.get("per_trace_scores")
    if type(scores) is not dict:
        return False
    novel = scores.get("novel")
    duplicate = scores.get("duplicate")
    if type(novel) is not list or not novel:
        return False
    if type(duplicate) is not list or not duplicate:
        return False
    if any(not is_finite_json_number(value) for value in novel):
        return False
    if any(
        value is not None and not is_finite_json_number(value)
        for value in duplicate
    ):
        return False
    if not any(value is not None for value in duplicate):
        return False

    if version == 3:
        passed_baseline_dominance = candidate.get("passed_baseline_dominance")
        dropped_rows = (
            candidate.get("dropped_novel_rows"),
            candidate.get("dropped_duplicate_rows"),
            candidate.get("dropped_paraphrase_rows"),
        )
        if type(passed_baseline_dominance) is not bool:
            return False
        if any(
            type(value) is not int or value < 0 or value > U64_MAX
            for value in dropped_rows
        ):
            return False
        recomputed = not any(value != 0 for value in dropped_rows) and (
            clears_required_auc(auc, required_auc)
        )
        # The persisted boolean is an auditable producer claim. Treat any
        # disagreement with the evidence-derived predicate as malformed and
        # fail closed; it is never the authority for eligibility.
        if passed_baseline_dominance != recomputed or not recomputed:
            return False

    return True


def pick_calibration_candidate(report, candidate_id=None):
    """Worst-of-passing: lowest AUC among candidates that:
       - AUC > 0.5
       - passed_determinism_gate = true
       - have non-null per_trace_scores
       - have no null entries in the novel slice (zero score failures)
       - under decision-rule v3, the report has a valid winner and the
         candidate's counters and AUC recompute as baseline-dominant
    """
    version = decision_rule_version(report)
    if version is None:
        return None
    partial = report.get("partial", False)
    if type(partial) is not bool or partial:
        return None
    candidates = report.get("candidates")
    if type(candidates) is not list:
        return None
    required_auc = None
    if version == 3:
        winner_id = report.get("winner_id")
        if type(winner_id) is not str or not winner_id:
            return None
        if not any(
            type(candidate) is dict and candidate.get("id") == winner_id
            for candidate in candidates
        ):
            return None
        baselines = report.get("baselines")
        if type(baselines) is not dict:
            return None
        required_auc = baselines.get("required_discrimination_auc")
        if not is_finite_json_number(required_auc):
            return None
    eligible = []
    for candidate in candidates:
        if type(candidate) is not dict:
            continue
        if candidate_id is not None and candidate.get("id") != candidate_id:
            continue
        if candidate_is_eligible(candidate, version, required_auc):
            eligible.append(candidate)
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
                    help="select a specific eligible candidate by id")
    args = ap.parse_args()

    with open(args.report) as f:
        report = json.load(f)

    if decision_rule_version(report) is None:
        print("error: invalid or unsupported decision_rule_version", file=sys.stderr)
        sys.exit(2)

    if args.candidate:
        candidates = report.get("candidates")
        exists = type(candidates) is list and any(
            type(candidate) is dict and candidate.get("id") == args.candidate
            for candidate in candidates
        )
        if not exists:
            print(f"error: candidate '{args.candidate}' not in report",
                  file=sys.stderr)
            sys.exit(2)
        # Explicit selection chooses among eligible candidates. This command
        # emits a deployable floor, so selecting an id is not an eligibility
        # override and has no force mode.
        target = pick_calibration_candidate(report, args.candidate)
        if not target:
            print(f"error: candidate '{args.candidate}' is not eligible for "
                  "calibration", file=sys.stderr)
            sys.exit(2)
    else:
        target = pick_calibration_candidate(report)
        if not target:
            print("error: no eligible calibration candidate "
                  "(need a complete report and version-appropriate "
                  "candidate evidence)",
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
