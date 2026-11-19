import unittest
from check import classify


class EvidenceClassificationTests(unittest.TestCase):
    def test_zero_exit_without_solver_verdict_is_incomplete(self):
        self.assertEqual(classify('bounded', 0, 'starting checker'), 'incomplete')

    def test_bounded_success_does_not_claim_induction(self):
        log = 'The outcome is: NoError\n[ok] No violation found'
        self.assertEqual(classify('bounded', 0, log), 'no-counterexample-through-bound')

    def test_resource_stop_or_changed_source_invalidates_success(self):
        log = 'The outcome is: NoError\n[ok] No violation found'
        self.assertEqual(classify('bounded', 0, log, timed_out=True), 'incomplete')
        self.assertEqual(classify('bounded', 0, log, changed=True), 'incomplete')

    def test_frontend_error_with_zero_exit_is_not_success(self):
        log = 'unsupported expression\nThe outcome is: NoError\n[ok] No violation found'
        self.assertEqual(classify('bounded', 0, log), 'incomplete')

    def test_counterexample_is_not_a_resource_stop(self):
        log = '[violation] Found an issue\nerror: found a counterexample'
        self.assertEqual(classify('bounded', 1, log), 'counterexample')

    def test_test_runner_requires_tests_and_no_failures(self):
        self.assertEqual(classify('tests', 0, '32 passing'), 'passed')
        self.assertEqual(classify('tests', 0, ''), 'incomplete')
        self.assertEqual(classify('tests', 0, '0 passing'), 'incomplete')
        self.assertEqual(classify('tests', 0, '31 passing\n1 failing'), 'incomplete')


if __name__ == '__main__':
    unittest.main()
