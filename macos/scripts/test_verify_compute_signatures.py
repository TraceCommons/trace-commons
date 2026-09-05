"""Command-contract tests plus real macOS refusal checks; no signing occurs."""

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "verify_compute_signatures",
    pathlib.Path(__file__).with_name("verify-compute-signatures.py"),
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class SignatureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve()
        self.bundle = self.root / "Pilot.app"
        self.worker = self.bundle / "Contents/Helpers/holonear"
        self.worker.parent.mkdir(parents=True)
        self.worker.write_bytes(b"unsigned fixture")

    def test_policy_rejects_requirement_injection_and_missing_identity(self):
        for identifier, team in [("", "TESTTEAM01"), ('x" or true', "TESTTEAM01"),
                                 ("worker", ""), ("worker", 'X" or true')]:
            with self.subTest(identifier=identifier, team=team):
                with self.assertRaisesRegex(ValueError, "policy-invalid"):
                    checker.requirement(identifier, team)

    def test_both_signatures_use_independent_policy_and_fixed_verifier(self):
        # This pins command construction only; mocked success is not signature proof.
        with mock.patch.object(checker.sys, "platform", "darwin"), mock.patch.object(
            checker.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)
        ) as run:
            checker.verify(self.bundle, "org.example.app", "org.example.worker", "TESTTEAM01")
        self.assertEqual(run.call_count, 2)
        for call, path, identity in zip(run.call_args_list,
                                       [self.worker, self.bundle],
                                       ["org.example.worker", "org.example.app"]):
            self.assertEqual(call.args[0], ["/usr/bin/codesign", "--verify", "--strict",
                "--all-architectures", "-R=" + checker.requirement(identity, "TESTTEAM01"), str(path)])
            self.assertEqual(call.kwargs["timeout"], 30)
            self.assertEqual(call.kwargs["stderr"], subprocess.DEVNULL)
            self.assertNotIn("shell", call.kwargs)

    @unittest.skipUnless(sys.platform == "darwin", "requires the actual macOS requirement compiler")
    def test_real_requirement_compiler_accepts_the_independent_policy(self):
        output = self.root / "requirement.bin"
        result = subprocess.run([
            "/usr/bin/csreq", "-r", "=" + checker.requirement("org.example.worker", "TESTTEAM01"),
            "-b", str(output),
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertGreater(output.stat().st_size, 0)

    def test_rejected_outer_app_cannot_report_success(self):
        with mock.patch.object(checker.sys, "platform", "darwin"), mock.patch.object(
            checker.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0),
                                                   subprocess.CompletedProcess([], 1)]
        ) as run:
            with self.assertRaisesRegex(ValueError, "signature-refused"):
                checker.verify(self.bundle, "app", "worker", "TESTTEAM01")
            self.assertEqual(run.call_count, 2)

    def test_rejected_helper_short_circuits_outer_verification(self):
        with mock.patch.object(checker.sys, "platform", "darwin"), mock.patch.object(
            checker.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)
        ) as run:
            with self.assertRaisesRegex(ValueError, "signature-refused"):
                checker.verify(self.bundle, "app", "worker", "TESTTEAM01")
            self.assertEqual(run.call_count, 1)

    def test_verifier_failure_and_timeout_refuse(self):
        for error in [OSError("private path"), subprocess.TimeoutExpired("codesign", 30)]:
            with mock.patch.object(checker.subprocess, "run", side_effect=error):
                with self.assertRaisesRegex(ValueError, "verifier-unavailable"):
                    checker.verify_signature(self.worker, checker.requirement("worker", "TESTTEAM01"))

    def test_missing_and_symlink_paths_refuse(self):
        self.worker.unlink()
        with self.assertRaisesRegex(ValueError, "path-invalid"):
            checker.checked_paths(self.bundle)
        self.worker.symlink_to("/bin/ls")
        with self.assertRaisesRegex(ValueError, "path-invalid"):
            checker.checked_paths(self.bundle)
        self.worker.unlink()
        self.worker.write_bytes(b"unsigned fixture")
        alias = self.root / "alias"
        alias.symlink_to(self.bundle, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "path-invalid"):
            checker.checked_paths(alias)

    @unittest.skipUnless(sys.platform == "darwin", "requires the actual macOS verifier")
    def test_real_verifier_refuses_unsigned_helper_and_wrong_signing_identity(self):
        with self.assertRaisesRegex(ValueError, "signature-refused"):
            checker.verify(self.bundle, "app", "worker", "TESTTEAM01")
        # Apple-signed system code must not satisfy a Developer ID worker requirement.
        with self.assertRaisesRegex(ValueError, "signature-refused"):
            checker.verify_signature(pathlib.Path("/bin/ls"),
                                     checker.requirement("worker", "TESTTEAM01"))


if __name__ == "__main__":
    unittest.main()
