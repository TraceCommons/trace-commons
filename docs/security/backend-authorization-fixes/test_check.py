"""Security of the evidence classifier: process success alone never proves a check."""
import unittest
from check import classify


class EvidenceClassification(unittest.TestCase):
    def classify(self, client='', server='', code=0, **kw):
        return classify(client, server, code, kw.get('timeout', False), kw.get('changed', False))

    def test_completed_bounded_check(self):
        self.assertEqual(self.classify('[ok] No violation found', 'The outcome is: NoError'),
                         'bounded-no-counterexample')

    def test_exit_zero_without_solver_completion_is_incomplete(self):
        self.assertEqual(self.classify('[ok] No violation found'), 'incomplete')

    def test_bare_exit_zero_is_incomplete(self):
        self.assertEqual(self.classify(), 'incomplete')

    def test_unsupported_expression_is_incomplete_even_with_success_markers(self):
        self.assertEqual(self.classify('[ok] No violation found',
            'unsupported expression\nThe outcome is: NoError'), 'incomplete')

    def test_timeout_cannot_prove(self):
        self.assertEqual(self.classify('[ok] No violation found', 'The outcome is: NoError', timeout=True), 'incomplete')

    def test_source_change_cannot_prove(self):
        self.assertEqual(self.classify('[ok] No violation found', 'The outcome is: NoError', changed=True), 'incomplete')

    def test_counterexample_requires_exit_one(self):
        log = '[violation] Found an issue\nerror: found a counterexample'
        self.assertEqual(self.classify(log, code=1), 'counterexample')
        self.assertEqual(self.classify(log, code=0), 'incomplete')

    def test_error_record_field_is_not_tool_diagnostic(self):
        log = '      error: "denied"\n[violation] Found an issue\nerror: found a counterexample'
        self.assertEqual(self.classify(log, code=1), 'counterexample')


if __name__ == '__main__':
    unittest.main()
