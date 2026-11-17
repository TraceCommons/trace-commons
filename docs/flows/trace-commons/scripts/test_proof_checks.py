#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest
from proof_checks import report_results, selected_for


class ProofChecks(unittest.TestCase):
    def test_only_explicit_verdicts_count(self):
        text = '# probe\n## P1 breaks\nA path\n## Rules that hold\nP2, NoDeadlock\n'
        self.assertEqual(report_results(text, ['P1', 'P2', 'P3'], exit_code=1), [
            {'definition': 'P1', 'status': 'breaks'},
            {'definition': 'P2', 'status': 'holds'},
            {'definition': 'P3', 'status': 'not checked'}])

    def test_prose_cannot_become_a_hold(self):
        text = '## Rules that hold\nP1\n\nP2 appears in explanatory prose.\n'
        self.assertEqual(report_results(text, ['P2'], exit_code=0)[0]['status'], 'not checked')

    def test_timeout_or_tool_error_is_never_a_proof(self):
        text = '## Rules that hold\nP1\n'
        self.assertEqual(report_results(text, ['P1'], timed_out=True, exit_code=-15)[0]['status'], 'not checked')
        self.assertEqual(report_results(text, ['P1'], exit_code=2)[0]['status'], 'not checked')
        self.assertEqual(report_results('', ['P1'], exit_code=0)[0]['status'], 'not checked')

    def test_selection_and_config_are_exact(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'models').mkdir()
            selection = {'status': 'selected', 'ownerSelection': {'request': 'all'},
                         'promises': [{'selected': True, 'model': 'sample', 'definition': 'P1'}]}
            (root / 'promise-selection.json').write_text(json.dumps(selection))
            cfg = root / 'models/sample.cfg'
            cfg.write_text('INIT q_init\nNEXT q_step\nINVARIANT P1\n')
            self.assertEqual(len(selected_for(root, 'sample')), 1)
            cfg.write_text('INIT q_init\nNEXT q_step\nINVARIANT P2\n')
            with self.assertRaises(ValueError):
                selected_for(root, 'sample')
            cfg.write_text('INVARIANT P1\nCONSTRAINT Small\n')
            with self.assertRaises(ValueError):
                selected_for(root, 'sample')
            selection['ownerSelection'] = None
            (root / 'promise-selection.json').write_text(json.dumps(selection))
            with self.assertRaises(ValueError):
                selected_for(root, 'sample')


if __name__ == '__main__':
    unittest.main()
