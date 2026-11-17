"""Reviewed exceptions must not weaken failures from the cargo-audit wrapper."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[3]
LOCKFILES = (
    "Cargo.lock",
    "crates/trace-commons-contributor-gtk/Cargo.lock",
    "tauri-desktop/Cargo.lock",
)


class CargoAuditPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.bin = Path(self.temporary.name)
        cargo = self.bin / "cargo"
        # Replace only the external cargo command. The real wrapper reads the
        # policy, verifies the Tauri backport and forwards cargo's exit status.
        cargo.write_text('''#!/usr/bin/env python3
import json
import os
import sys

args = sys.argv[1:]
print(json.dumps(args))
if "AUDIT_TEST_EXIT" in os.environ:
    sys.exit(int(os.environ["AUDIT_TEST_EXIT"]))
ignored = {args[i + 1] for i, arg in enumerate(args[:-1]) if arg == "--ignore"}
finding = os.environ["AUDIT_TEST_FINDING"]
denied = any(args[i:i + 2] == ["--deny", "warnings"] for i in range(len(args)))
sys.exit(23 if finding not in ignored and denied else 0)
''')
        cargo.chmod(0o755)

    def run_wrapper(self, lockfile, *, finding="RUSTSEC-2099-9999", exit_code=None):
        environment = os.environ.copy()
        environment["PATH"] = str(self.bin) + os.pathsep + environment.get("PATH", "")
        environment["AUDIT_TEST_FINDING"] = finding
        if exit_code is not None:
            environment["AUDIT_TEST_EXIT"] = str(exit_code)
        return subprocess.run(
            [sys.executable, str(ROOT / "scripts/ci/cargo-audit.py"),
             "--file", lockfile, "--json", "--no-fetch"],
            cwd=self.bin, env=environment, text=True, capture_output=True,
            check=False,
        )

    def test_reviewed_maintenance_findings_are_accepted_in_each_workspace(self):
        for lockfile in LOCKFILES:
            for advisory in ("RUSTSEC-2026-0543", "RUSTSEC-2026-0555"):
                with self.subTest(lockfile=lockfile, advisory=advisory):
                    result = self.run_wrapper(lockfile, finding=advisory)
                    self.assertEqual(result.returncode, 0, result.stderr)

    def test_unreviewed_warning_still_fails_in_each_workspace(self):
        for lockfile in LOCKFILES:
            with self.subTest(lockfile=lockfile):
                result = self.run_wrapper(lockfile)
                self.assertEqual(result.returncode, 23, result.stderr)

    def test_cargo_failure_status_is_preserved(self):
        for status in (1, 2, 101):
            with self.subTest(status=status):
                result = self.run_wrapper("Cargo.lock", exit_code=status)
                self.assertEqual(result.returncode, status, result.stderr)

    def test_lockfile_and_options_reach_cargo_unchanged(self):
        for lockfile in LOCKFILES:
            with self.subTest(lockfile=lockfile):
                result = self.run_wrapper(lockfile, exit_code=0)
                self.assertEqual(result.returncode, 0, result.stderr)
                command = json.loads(result.stdout)
                self.assertEqual(command[:3], ["audit", "--deny", "warnings"])
                self.assertEqual(command[-4:], ["--file", lockfile, "--json", "--no-fetch"])
                ignored = {command[i + 1] for i, arg in enumerate(command[:-1])
                           if arg == "--ignore"}
                self.assertEqual("RUSTSEC-2024-0429" in ignored,
                                 lockfile == "tauri-desktop/Cargo.lock")


if __name__ == "__main__":
    unittest.main()
