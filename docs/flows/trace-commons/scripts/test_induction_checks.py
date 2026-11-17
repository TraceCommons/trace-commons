import unittest
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from induction_checks import classify_obligation, certificate_status


class InductionChecks(unittest.TestCase):
    def test_complete_success_requires_terminal_tool_verdict(self):
        log = 'The outcome is: NoError I@12:00\n[ok] No violation found (12ms).\n'
        self.assertEqual(classify_obligation(log, 0), 'proved')
        self.assertEqual(classify_obligation('[ok] No violation found', 0), 'incomplete')
        self.assertEqual(classify_obligation('State 1: invariant holds', 0), 'incomplete')

    def test_zero_exit_with_tool_error_is_not_a_proof(self):
        log = 'The outcome is: NoError\nunsupported expression E@12:00\n[ok] No violation found\n'
        self.assertEqual(classify_obligation(log, 0), 'incomplete')
        self.assertEqual(classify_obligation('error: connection dropped', 0), 'incomplete')

    def test_timeout_and_changed_inputs_invalidate_success(self):
        log = 'The outcome is: NoError\n[ok] No violation found\n'
        self.assertEqual(classify_obligation(log, 0, timed_out=True), 'incomplete')
        self.assertEqual(classify_obligation(log, 0, artifacts_changed=True), 'incomplete')
        self.assertEqual(classify_obligation(log, -15), 'incomplete')

    def test_counterexample_is_an_obligation_result_not_product_verdict(self):
        log = 'The outcome is: Error\n[violation] Found an issue\nerror: found a counterexample\n'
        self.assertEqual(classify_obligation(log, 1), 'counterexample')
        self.assertEqual(classify_obligation(log, 1, timed_out=True), 'incomplete')

    def test_counterexample_record_error_field_is_not_a_tool_failure(self):
        log = 'The outcome is: Error\ns = {\n      error: "YouClearAccountHold",\n}\n[violation] Found an issue\nerror: found a counterexample\n'
        self.assertEqual(classify_obligation(log, 1), 'counterexample')

    def test_mixed_counterexample_and_tool_error_is_incomplete(self):
        log = '[violation] Found an issue\nerror: found a counterexample\nUnsupportedOperationException\n'
        self.assertEqual(classify_obligation(log, 1), 'incomplete')

    def test_empty_spec_cannot_report_a_completed_proof(self):
        with tempfile.TemporaryDirectory() as tmp:
            spec = Path(tmp) / 'empty.json'
            spec.write_text(json.dumps({'models': []}))
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('run_induction.py')),
                                     '--spec', str(spec), '--out', str(Path(tmp) / 'results')],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('at least one', result.stderr)

    def phases(self):
        return [{'phase': phase, 'status': 'counterexample' if phase == 'nonvacuity' else 'proved',
                 'modelSha256': 'model', 'harnessSha256': 'harness'}
                for phase in ['initiation', 'consecution', 'implication', 'nonvacuity']]

    def test_all_obligations_and_nonempty_domain_are_required(self):
        phases = self.phases()
        self.assertEqual(certificate_status(phases), 'proved')
        self.assertEqual(certificate_status(phases[:-1]), 'incomplete')
        phases[-1]['status'] = 'proved'  # false held: initializer was empty
        self.assertEqual(certificate_status(phases), 'incomplete')

    def test_stale_or_duplicate_phase_cannot_complete_a_certificate(self):
        phases = self.phases()
        phases[1]['harnessSha256'] = 'old-harness'
        self.assertEqual(certificate_status(phases), 'incomplete')
        phases = self.phases()
        self.assertEqual(certificate_status(phases + [phases[0]]), 'incomplete')
        phases[0]['modelSha256'] = ''
        self.assertEqual(certificate_status(phases), 'incomplete')
