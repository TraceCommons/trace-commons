#!/usr/bin/env python3

import json
import math
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


def baselines(required_discrimination_auc=0.85):
    return {"required_discrimination_auc": required_discrimination_auc}


def run_cli(report, *args):
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json") as report_file:
        json.dump(report, report_file)
        report_file.flush()
        return subprocess.run(
            [sys.executable, calibration.__file__, report_file.name, *args],
            check=False,
            capture_output=True,
            text=True,
        )


class PickCalibrationCandidateTests(unittest.TestCase):
    def test_v3_report_without_winner_rejects_all_candidates(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": None,
            "baselines": baselines(),
            "candidates": [candidate()],
        }

        self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_v3_requires_baseline_flag_and_complete_discrimination_support(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": "eligible",
            "baselines": baselines(),
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
                    "baselines": baselines(),
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
                    "baselines": baselines(),
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
            "baselines": baselines(),
            "candidates": [candidate()],
        }

        self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_invalid_version_exits_through_controlled_error_path(self):
        report = {
            "decision_rule_version": None,
            "winner_id": "candidate",
            "baselines": baselines(),
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

    def test_cli_rejects_coerced_v3_eligibility_evidence(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": "coerced",
            "baselines": baselines(),
            "candidates": [{
                "id": "coerced",
                "discrimination_auc": 0.9,
                "passed_determinism_gate": True,
                "passed_baseline_dominance": "false",
                "dropped_novel_rows": False,
                "dropped_duplicate_rows": 0.0,
                "dropped_paraphrase_rows": 0.0,
                "per_trace_scores": {
                    "novel": [2.0, 3.0],
                    "duplicate": [1.0, 1.0],
                },
            }],
        }
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json") as report_file:
            json.dump(report, report_file)
            report_file.flush()
            completed = subprocess.run(
                [sys.executable, calibration.__file__, report_file.name],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(completed.returncode, 2)
        self.assertIn("error: no eligible calibration candidate", completed.stderr)
        self.assertNotIn(
            "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=", completed.stdout
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

    def test_explicit_candidate_cannot_bypass_v3_eligibility(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": None,
            "partial": False,
            "baselines": baselines(),
            "candidates": [candidate(passed_baseline_dominance=False)],
        }

        completed = run_cli(report, "--candidate", "candidate")

        self.assertEqual(completed.returncode, 2)
        self.assertIn("candidate 'candidate' is not eligible", completed.stderr)
        self.assertNotIn(
            "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=", completed.stdout
        )

    def test_full_boolean_and_numeric_field_set_requires_exact_json_types(self):
        def complete_report():
            return {
                "decision_rule_version": 3,
                "winner_id": "candidate",
                "partial": False,
                "baselines": baselines(),
                "candidates": [candidate()],
            }

        malformed = [
            ("decision_rule_version_bool", ("decision_rule_version",), True),
            ("decision_rule_version_float", ("decision_rule_version",), 3.0),
            ("decision_rule_version_string", ("decision_rule_version",), "3"),
            ("partial", ("partial",), "false"),
            ("discrimination_auc", ("candidates", 0, "discrimination_auc"), True),
            (
                "passed_determinism_gate",
                ("candidates", 0, "passed_determinism_gate"),
                "false",
            ),
            (
                "passed_baseline_dominance",
                ("candidates", 0, "passed_baseline_dominance"),
                1,
            ),
            ("dropped_novel_rows", ("candidates", 0, "dropped_novel_rows"), False),
            (
                "dropped_duplicate_rows",
                ("candidates", 0, "dropped_duplicate_rows"),
                0.0,
            ),
            (
                "dropped_paraphrase_rows",
                ("candidates", 0, "dropped_paraphrase_rows"),
                "0",
            ),
            (
                "novel_score",
                ("candidates", 0, "per_trace_scores", "novel", 0),
                True,
            ),
            (
                "duplicate_score",
                ("candidates", 0, "per_trace_scores", "duplicate", 0),
                "1.0",
            ),
        ]

        for field, path, value in malformed:
            with self.subTest(field=field):
                report = complete_report()
                target = report
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                self.assertIsNone(calibration.pick_calibration_candidate(report))

    def test_partial_report_is_rejected_for_every_supported_version(self):
        for version in (1, 2, 3):
            with self.subTest(version=version):
                report = {
                    "decision_rule_version": version,
                    "winner_id": "candidate" if version == 3 else None,
                    "partial": True,
                    "baselines": baselines(),
                    "candidates": [candidate()],
                }
                completed = run_cli(report)
                self.assertEqual(completed.returncode, 2)
                self.assertIn(
                    "error: no eligible calibration candidate", completed.stderr
                )
                self.assertNotIn(
                    "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=",
                    completed.stdout,
                )

    def test_well_formed_complete_v3_report_emits_floor(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": "candidate",
            "partial": False,
            "baselines": baselines(),
            "candidates": [candidate()],
        }

        completed = run_cli(report)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn(
            "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=", completed.stdout
        )

    def test_v3_recomputes_baseline_dominance_instead_of_trusting_flag(self):
        report = {
            "decision_rule_version": 3,
            "winner_id": "forged",
            "partial": False,
            "baselines": baselines(0.95),
            "candidates": [
                candidate(
                    id="forged",
                    discrimination_auc=0.9,
                    passed_baseline_dominance=True,
                )
            ],
        }

        completed = run_cli(report)

        self.assertEqual(completed.returncode, 2)
        self.assertIn("error: no eligible calibration candidate", completed.stderr)
        self.assertNotIn(
            "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=", completed.stdout
        )
        self.assertNotIn("Traceback", completed.stderr)

    def test_v3_baseline_recomputation_matches_four_ulp_boundary(self):
        required = 0.9
        within = required
        for _ in range(4):
            within = math.nextafter(within, -math.inf)
        outside = math.nextafter(within, -math.inf)

        within_report = {
            "decision_rule_version": 3,
            "winner_id": "within",
            "partial": False,
            "baselines": baselines(required),
            "candidates": [candidate(id="within", discrimination_auc=within)],
        }
        outside_report = {
            "decision_rule_version": 3,
            "winner_id": "outside",
            "partial": False,
            "baselines": baselines(required),
            "candidates": [candidate(id="outside", discrimination_auc=outside)],
        }

        self.assertEqual(
            calibration.pick_calibration_candidate(within_report)["id"], "within"
        )
        self.assertIsNone(calibration.pick_calibration_candidate(outside_report))

    def test_v3_winner_id_must_be_a_string_naming_a_report_candidate(self):
        for winner_id in ("", 0, True, "does-not-exist"):
            with self.subTest(winner_id=winner_id):
                report = {
                    "decision_rule_version": 3,
                    "winner_id": winner_id,
                    "partial": False,
                    "baselines": baselines(),
                    "candidates": [candidate()],
                }

                completed = run_cli(report)

                self.assertEqual(completed.returncode, 2)
                self.assertIn(
                    "error: no eligible calibration candidate", completed.stderr
                )
                self.assertNotIn(
                    "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=",
                    completed.stdout,
                )
                self.assertNotIn("Traceback", completed.stderr)


if __name__ == "__main__":
    unittest.main()
