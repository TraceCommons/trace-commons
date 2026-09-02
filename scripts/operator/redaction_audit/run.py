#!/usr/bin/env python3
"""Score the synthetic conformance corpus with the independent auditor."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import redaction_audit  # noqa: E402


CASE_KEYS = {
    "case_id",
    "input_format",
    "record",
    "forbidden_classes",
    "expected_disposition",
}
FORMATS = {
    "synthetic_claude_jsonl",
    "synthetic_codex_jsonl",
    "synthetic_path_manifest",
}
CLASSES = {"identity", "legal_matter", "third_party_pii", "personal", "secret"}
DISPOSITIONS = {"drop", "normalize", "keep"}
CASE_ID_RE = re.compile(r"synthetic-fuzz-\d{3}\Z")


def validate_case(case: Any, line_number: int) -> list[str]:
    errors: list[str] = []
    if not isinstance(case, dict):
        return [f"line {line_number}: case must be an object"]
    if set(case) != CASE_KEYS:
        errors.append(f"line {line_number}: fields do not match the schema")
    case_id = case.get("case_id")
    if not isinstance(case_id, str) or CASE_ID_RE.fullmatch(case_id) is None:
        errors.append(f"line {line_number}: invalid case_id")
    if case.get("input_format") not in FORMATS:
        errors.append(f"line {line_number}: invalid input_format")
    record = case.get("record")
    if not isinstance(record, dict) or record.get("fixture_notice") != "SYNTHETIC_FUZZ_FIXTURE":
        errors.append(f"line {line_number}: record lacks the synthetic fixture marker")
    forbidden = case.get("forbidden_classes")
    if (
        not isinstance(forbidden, list)
        or not forbidden
        or len(forbidden) != len(set(forbidden))
        or any(item not in CLASSES for item in forbidden)
    ):
        errors.append(f"line {line_number}: invalid forbidden_classes")
    if case.get("expected_disposition") not in DISPOSITIONS:
        errors.append(f"line {line_number}: invalid expected_disposition")
    return errors


def load_cases(path: Path) -> tuple[list[dict[str, Any]], list[str]]:
    cases: list[dict[str, Any]] = []
    errors: list[str] = []
    seen_ids: set[str] = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            errors.append(f"line {line_number}: blank lines are not allowed")
            continue
        try:
            case = json.loads(line)
        except json.JSONDecodeError:
            errors.append(f"line {line_number}: invalid JSON")
            continue
        errors.extend(validate_case(case, line_number))
        case_id = case.get("case_id") if isinstance(case, dict) else None
        if isinstance(case_id, str):
            if case_id in seen_ids:
                errors.append(f"line {line_number}: duplicate case_id")
            seen_ids.add(case_id)
        if isinstance(case, dict):
            cases.append(case)
    return cases, errors


def score(path: Path) -> tuple[dict[str, Any], int]:
    cases, validation_errors = load_cases(path)
    passed = 0
    failures: list[dict[str, Any]] = []
    class_totals: Counter[str] = Counter()
    class_passed: Counter[str] = Counter()
    negative_cases = 0
    # Confusion-matrix counters, at case granularity:
    #   positive case (expected_disposition != "keep") that the auditor correctly
    #   flagged -> true positive; one it missed -> false negative.
    #   negative case (expected_disposition == "keep") the auditor correctly left
    #   alone -> true negative; one it wrongly flagged -> false positive.
    true_positives = 0
    false_negatives = 0
    true_negatives = 0
    false_positives = 0

    with tempfile.TemporaryDirectory(prefix="trace-fuzz-score-", dir="/tmp") as temp_name:
        temp_root = Path(temp_name)
        for case in cases:
            case_id = case["case_id"]
            case_path = temp_root / f"{case_id}.jsonl"
            case_path.write_text(json.dumps(case["record"], sort_keys=True) + "\n", encoding="utf-8")
            report = redaction_audit.audit(case_path)
            observed = {item["forbidden_class"] for item in report["detections"]}
            expected = set(case["forbidden_classes"])
            for item in expected:
                class_totals[item] += 1
            is_negative = case["expected_disposition"] == "keep"
            if is_negative:
                negative_cases += 1
                ok = not observed and not report["failures"]
                if ok:
                    true_negatives += 1
                else:
                    false_positives += 1
            else:
                ok = expected.issubset(observed) and not report["failures"]
                if ok:
                    true_positives += 1
                else:
                    false_negatives += 1
            if ok:
                passed += 1
                for item in expected:
                    class_passed[item] += 1
            else:
                failures.append(
                    {
                        "case_id": case_id,
                        "kind": "false_positive" if is_negative else "false_negative",
                        "expected": sorted(expected),
                        "observed": sorted(observed),
                        "audit_failures": len(report["failures"]),
                    }
                )

    precision = (
        round(true_positives / (true_positives + false_positives), 4)
        if (true_positives + false_positives) > 0
        else None
    )
    recall = (
        round(true_positives / (true_positives + false_negatives), 4)
        if (true_positives + false_negatives) > 0
        else None
    )

    result = {
        "surface": "redaction_audit.audit(record-as-jsonl)",
        "score": f"{passed}/{len(cases)}",
        "passed": passed,
        "total": len(cases),
        "negative_cases": negative_cases,
        "positive_cases": len(cases) - negative_cases,
        "true_positives": true_positives,
        "false_negatives": false_negatives,
        "true_negatives": true_negatives,
        "false_positives": false_positives,
        "precision": precision,
        "recall": recall,
        "precision_fraction": (
            f"{true_positives}/{true_positives + false_positives}"
            if (true_positives + false_positives) > 0
            else "unscored_no_flagged_cases"
        ),
        "recall_fraction": (
            f"{true_positives}/{true_positives + false_negatives}"
            if (true_positives + false_negatives) > 0
            else "unscored_no_positive_cases"
        ),
        "class_scores": {
            name: f"{class_passed[name]}/{class_totals[name]}" for name in sorted(class_totals)
        },
        "schema_errors": validation_errors,
        "failures": failures,
    }
    exit_code = 0 if not validation_errors and passed == len(cases) else 1
    return result, exit_code


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Score synthetic trace fuzz cases.")
    parser.add_argument(
        "path",
        nargs="?",
        type=Path,
        default=Path(__file__).with_name("cases.jsonl"),
        help="JSONL corpus path",
    )
    args = parser.parse_args(argv)
    result, exit_code = score(args.path)
    print(json.dumps(result, indent=2, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
