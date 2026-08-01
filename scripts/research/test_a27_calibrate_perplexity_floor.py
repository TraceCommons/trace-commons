#!/usr/bin/env python3

import json
import subprocess
import sys
import tempfile
import unittest

import a27_calibrate_perplexity_floor as calibration


def candidate(**overrides):
    row = {
        "id": "candidate",
        "discrimination_auc": 0.9,
        "passed_determinism_gate": True,
        "passed_baseline_dominance": True,
        "dropped_novel_rows": 0,
        "dropped_duplicate_rows": 0,
        "dropped_paraphrase_rows": 0,
        "per_trace_scores": {
            "novel": [2.0, 3.0],
            "duplicate": [1.0, 1.0],
        },
    }
    row.update(overrides)
    return row


class PickCalibrationCandidateTests(unittest.TestCase):
    def test_v3_report_without_winner_rejects_all_candidates(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": None,
            "candidates": [candidate()],
        }

        self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_v3_requires_baseline_flag_and_complete_discrimination_support(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": "eligible",
            "candidates": [
                candidate(id="failed-baseline", passed_baseline_dominance=False),
                candidate(id="dropped-novel", dropped_novel_rows=1),
                candidate(id="dropped-duplicate", dropped_duplicate_rows=1),
                candidate(id="dropped-paraphrase", dropped_paraphrase_rows=1),
                candidate(id="eligible", discrimination_auc=0.95),
            ],
        }

        selected = calibration.pick_calibration_candidate(report)
        self.assertEqual(selected["id"], "eligible")

    def test_v3_requires_explicit_drop_evidence(self):
        for field in (
            "dropped_novel_rows",
            "dropped_duplicate_rows",
            "dropped_paraphrase_rows",
        ):
            with self.subTest(field=field):
                row = candidate()
                del row[field]
                report = {
                    "decision_rule_version": 3,
                    "winner_id": "candidate",
                    "candidates": [row],
                }

                self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_missing_and_non_integer_versions_are_rejected(self):
        invalid_versions = (None, "3", True, 2.5)
        for version in invalid_versions:
            with self.subTest(version=version):
                report = {
                    "decision_rule_version": version,
                    "winner_id": "candidate",
                    "candidates": [candidate()],
                }
                self.assertIsNone(calibration.pick_calibration_candidate(report))

        missing_version_report = {
            "winner_id": "candidate",
            "candidates": [candidate()],
        }
        self.assertIsNone(
            calibration.pick_calibration_candidate(missing_version_report)
        )

    def test_unknown_future_version_is_rejected(self):
        report = {
            "decision_rule_version": 4,
            "winner_id": "candidate",
            "candidates": [candidate()],
        }

        self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_invalid_version_exits_through_controlled_error_path(self):
        report = {
            "decision_rule_version": None,
            "winner_id": "candidate",
            "candidates": [candidate()],
        }
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json") as report_file:
            json.dump(report, report_file)
            report_file.flush()
            completed = subprocess.run(
                [
                    sys.executable,
                    calibration.__file__,
                    report_file.name,
                    "--candidate",
                    "candidate",
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(completed.returncode, 2)
        self.assertIn(
            "error: invalid or unsupported decision_rule_version", completed.stderr
        )
        self.assertNotIn("Traceback", completed.stderr)

    def test_v1_and_v2_keep_archived_eligibility_behavior(self):
        legacy = candidate(
            passed_baseline_dominance=False,
            dropped_novel_rows=7,
            dropped_duplicate_rows=9,
        )
        for version in (1, 2):
            with self.subTest(version=version):
                report = {
                    "decision_rule_version": version,
                    "winner_id": None,
                    "candidates": [legacy],
                }
                selected = calibration.pick_calibration_candidate(report)
                self.assertEqual(selected["id"], "candidate")


if __name__ == "__main__":
    unittest.main()
