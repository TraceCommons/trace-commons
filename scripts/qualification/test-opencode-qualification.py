#!/usr/bin/env python3
"""Mutation checks for the cheap guard; no binary, network or models required."""
import importlib.util
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("guard", HERE / "check-opencode-qualification.py")
GUARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GUARD)
ROOT = HERE.parents[1]


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.inputs = [
            (ROOT / "crates/trace-commons-contributor/src/source/opencode.rs").read_text(),
            (HERE / "opencode-routing.py").read_text(),
            (ROOT / "docs/superpowers/reports/2026-09-07-opencode-export-qualification.md").read_text(),
            (ROOT / "crates/trace-commons-contributor/src/daemon/harness.rs").read_text(),
        ]

    def test_current(self):
        self.assertEqual(GUARD.check(*self.inputs), "1.18.29")

    def test_reader_pin_drift(self):
        self.inputs[0] = self.inputs[0].replace('"1.18.29"', '"1.19.0"')
        with self.assertRaises(ValueError):
            GUARD.check(*self.inputs)

    def test_removed_runtime_refusal_despite_retained_output_pin(self):
        self.inputs[1] = self.inputs[1].replace(
            'if subprocess.check_output([str(args.opencode), "--version"], text=True).strip() != "1.18.29":\n'
            '    raise SystemExit("unqualified OpenCode version")\n', '')
        self.assertIn('"version": "1.18.29"', self.inputs[1])
        with self.assertRaises(ValueError):
            GUARD.check(*self.inputs)

    def test_non_refusing_version_comparison(self):
        self.inputs[1] = self.inputs[1].replace('raise SystemExit("unqualified OpenCode version")', 'pass')
        with self.assertRaises(ValueError):
            GUARD.check(*self.inputs)

    def test_push_after_empty_initialization(self):
        self.inputs[3] = self.inputs[3].replace(
            'fn owned_agents() -> Vec<AgentEntry> {\n    Vec::new()\n}',
            'fn owned_agents() -> Vec<AgentEntry> {\n    let mut agents = Vec::new();\n'
            '    agents.push(synthetic_entry());\n    agents\n}')
        self.assertIn('agents.push', self.inputs[3])
        with self.assertRaises(ValueError):
            GUARD.check(*self.inputs)


if __name__ == "__main__":
    unittest.main()
