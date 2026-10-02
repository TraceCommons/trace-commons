#!/usr/bin/env python3
"""Self-tests for `pipeline.py` and `pipeline_tooling`.

Standard library `unittest`, no Docker, no cargo: every subprocess call goes
through `pipeline_tooling.environment._invoke`, the sole child-process
boundary, and every test here replaces it with a fake. Run directly:

    python3 scripts/operator/test_pipeline_tooling.py
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import tempfile
import unittest
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest import mock

import pipeline
from pipeline_tooling import cargo, catalog, checks, corpus, environment, errors, results
from pipeline_tooling import report as report_module


def _fake_hash(label):
    return "sha256:" + hashlib.sha256(label.encode()).hexdigest()


def _iso(when):
    return when.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _scratch_run(prefix="selftest"):
    """A real `Run` rooted under the repository's own `.local/` (gitignored),
    so `StepFailed.log_path.relative_to(ROOT)` in `pipeline.main` resolves
    the way it does for a real invocation. Callers remove `run.run_dir`."""
    run_dir = environment.ROOT / ".local" / "pipeline" / "runs" / f"{prefix}-{uuid.uuid4().hex[:8]}"
    run_dir.mkdir(parents=True)
    (run_dir / "logs").mkdir()
    (run_dir / "results").mkdir()
    return environment.Run(
        run_id="q" + uuid.uuid4().hex[:8],
        started_at=datetime.now(timezone.utc) - timedelta(minutes=5),
        run_dir=run_dir,
        code_revision_hash=_fake_hash("code-revision"),
    )


def _make_run(tmp_path, run_id="qcafebabe", code_revision_hash=None, started_at=None):
    (tmp_path / "results").mkdir(parents=True, exist_ok=True)
    return environment.Run(
        run_id=run_id,
        started_at=started_at or (datetime.now(timezone.utc) - timedelta(minutes=5)),
        run_dir=tmp_path,
        code_revision_hash=code_revision_hash or _fake_hash("code-revision"),
    )


def _write_check_files(results_dir, check_id, evidence, **overrides):
    evidence_hash = "sha256:" + hashlib.sha256(results.canonical(evidence)).hexdigest()
    raw = {
        "schema": results.SCHEMA,
        "run_id": "qcafebabe",
        "check_id": check_id,
        "status": "pass",
        "code_revision_hash": _fake_hash("code-revision"),
        "package_hash": _fake_hash("package"),
        "configuration_digest": _fake_hash("configuration"),
        "dependency_digest": _fake_hash("dependency"),
        "observed_at": _iso(datetime.now(timezone.utc)),
        "evidence_hash": evidence_hash,
        "safe_blockers": [],
    }
    raw.update(overrides)
    (results_dir / f"{check_id}.result.json").write_text(json.dumps(raw))
    (results_dir / f"{check_id}.evidence.json").write_text(json.dumps(evidence))
    return raw


class ChildEnvironmentTests(unittest.TestCase):
    def test_child_environment_drops_ambient_database_urls(self):
        ambient_secrets = {
            "DATABASE_URL": "postgres://evil@example.invalid/db",
            "PGPASSWORD": "hunter2",
            "AWS_SECRET_ACCESS_KEY": "akia-fake-secret",
            "TRACE_COMMONS_TENANT_TOKENS": "tenant-token-blob",
        }
        with mock.patch.dict(os.environ, ambient_secrets):
            result = environment.child_environment(
                {"TRACE_COMMONS_PG_TEST_DATABASE_URL": "postgres://trace@127.0.0.1/db"}
            )
        self.assertIn("PATH", result)
        self.assertEqual(result["TRACE_COMMONS_PG_TEST_DATABASE_URL"], "postgres://trace@127.0.0.1/db")
        for key in ambient_secrets:
            self.assertNotIn(key, result)

        with self.assertRaises(errors.ToolingError) as ctx:
            environment.child_environment({"FOO": "bar"})
        self.assertEqual(str(ctx.exception), "child_environment_key_invalid")

        with mock.patch.dict(os.environ, {"RUSTFLAGS": "-D warnings"}):
            result = environment.child_environment({})
        self.assertEqual(result.get("RUSTFLAGS"), "-D warnings")

        with mock.patch.dict(os.environ):
            os.environ.pop("RUSTFLAGS", None)
            result = environment.child_environment({})
        self.assertNotIn("RUSTFLAGS", result)


class CargoZeroMatchTests(unittest.TestCase):
    def test_zero_match_filter_fails(self):
        run = _scratch_run()
        calls = []

        def fake_invoke_zero(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append(list(command))
            return (0, "running 0 tests\n") if capture else (0, None)

        try:
            with mock.patch.object(environment, "_invoke", fake_invoke_zero):
                with self.assertRaises(errors.ToolingError) as ctx:
                    cargo.cargo_test(run, "zero_step", ("-p", "example", "--lib"), "not_a_real_test", {})
            self.assertEqual(str(ctx.exception), "cargo_filter_matched_zero_tests")
            self.assertEqual(len(calls), 1, "the real command must not run after a zero-match list")
            self.assertIn("--list", calls[0])
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

        run = _scratch_run()
        calls = []

        def fake_invoke_match(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append(list(command))
            return (0, "example::a::b: test\n") if capture else (0, None)

        try:
            with mock.patch.object(environment, "_invoke", fake_invoke_match):
                cargo.cargo_test(
                    run, "match_step", ("-p", "example", "--lib"), "b", {}, exact=True, ignored=True
                )
            self.assertEqual(len(calls), 2, "exactly one list call and one real run")
            list_command, run_command = calls
            self.assertIn("--list", list_command)
            self.assertEqual(list_command[-2:], ["--exact", "--ignored"])
            self.assertNotIn("--list", run_command)
            self.assertEqual(run_command[-2:], ["--exact", "--ignored"])
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class CargoListFailureTests(unittest.TestCase):
    """Final review I5: a `--list` step that exits nonzero (a compile error)
    fails with its own label and the protected log holding its output --
    never as a zero-match filter -- and the real command never runs."""

    def test_a_failed_list_step_fails_with_its_own_label_and_log(self):
        run = _scratch_run()
        calls = []
        compiler_output = "error[E0425]: cannot find value `x` in this scope\n"

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append(list(command))
            return (101, compiler_output) if capture else (0, None)

        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with self.assertRaises(errors.StepFailed) as ctx:
                    cargo.cargo_test(run, "list_step", ("-p", "example", "--lib"), "some_test", {})
            failure = ctx.exception
            self.assertEqual(str(failure), "cargo_test_list_failed:list_step")
            self.assertEqual(failure.exit_code, 101)
            self.assertEqual(failure.log_path, run.run_dir / "logs" / "list_step.log")
            self.assertEqual(failure.log_path.read_text(), compiler_output)
            self.assertEqual(len(calls), 1, "the real command must not run after a failed list")

            def handler(args, r):
                with mock.patch.object(environment, "_invoke", fake_invoke):
                    cargo.cargo_test(r, "list_step", ("-p", "example", "--lib"), "some_test", {})

            with mock.patch.object(pipeline, "Run") as mock_run_cls, mock.patch.object(
                pipeline, "parse_args", return_value=argparse.Namespace(handler=handler)
            ):
                mock_run_cls.create.return_value = run
                stderr = io.StringIO()
                with contextlib.redirect_stderr(stderr):
                    code = pipeline.main([])
            self.assertEqual(code, 101)
            line = stderr.getvalue().strip()
            self.assertTrue(line.startswith("PipelineFailure: cargo_test_list_failed:list_step exit=101 log="), line)
            self.assertNotIn("E0425", line)
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class ObservedAtParsingTests(unittest.TestCase):
    """Final review I2: chrono writes nine fractional digits on Linux, which
    `datetime.fromisoformat` refuses before Python 3.11."""

    def test_nine_fraction_digits_parse_on_every_supported_python(self):
        expected = datetime(2026, 9, 30, 14, 57, 21, 810069, tzinfo=timezone.utc)
        for label, value in (
            ("z_suffix", "2026-09-30T14:57:21.810069123Z"),
            ("offset", "2026-09-30T15:57:21.810069123+01:00"),
            ("six_digits", "2026-09-30T14:57:21.810069Z"),
        ):
            with self.subTest(label):
                self.assertEqual(results._parse_observed_at(value), expected)
        self.assertEqual(
            results._parse_observed_at("2026-09-30T14:57:21.8Z"),
            datetime(2026, 9, 30, 14, 57, 21, 800000, tzinfo=timezone.utc),
        )
        self.assertEqual(
            results._parse_observed_at("2026-09-30T14:57:21Z"),
            datetime(2026, 9, 30, 14, 57, 21, tzinfo=timezone.utc),
        )
        # What `fromisoformat` receives: six digits, whatever Python runs.
        self.assertEqual(
            results._six_fraction_digits("2026-09-30T14:57:21.810069123+00:00"),
            "2026-09-30T14:57:21.810069+00:00",
        )
        for refused in ("2026-09-30T14:57:21.810069123", "not-a-time", 12):
            with self.subTest(refused=refused):
                with self.assertRaises(errors.ToolingError) as ctx:
                    results._parse_observed_at(refused)
                self.assertEqual(str(ctx.exception), "check_result_schema_invalid")

    def test_a_nine_digit_result_loads(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                observed_at=_iso(datetime.now(timezone.utc) - timedelta(seconds=1)).replace("Z", "123Z"),
            )
            loaded = results.load_results(run)
            results.require_current_pass_results(
                run, loaded, {"pipeline_crash_matrix": checks.CheckSpec("pipeline_crash_matrix", True)}
            )


class ResultsValidationTests(unittest.TestCase):
    def test_results_reject_missing_stale_foreign_and_tampered(self):
        required = {"pipeline_crash_matrix": checks.CheckSpec("pipeline_crash_matrix", digests_required=True)}

        with self.subTest("missing"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_missing:pipeline_crash_matrix")

        with self.subTest("foreign_run"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id="qnotthisrun", code_revision_hash=run.code_revision_hash,
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_foreign_run")

        with self.subTest("foreign_revision"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=_fake_hash("some-other-revision"),
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_foreign_revision")

        with self.subTest("stale_before_start"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                observed_at=_iso(run.started_at - timedelta(seconds=5)),
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_stale")

        with self.subTest("stale_future"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                observed_at=_iso(datetime.now(timezone.utc) + timedelta(hours=1)),
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_stale")

        with self.subTest("blocked"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash, status="blocked",
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_blocked:pipeline_crash_matrix")

        with self.subTest("failed"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash, status="fail",
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_failed:pipeline_crash_matrix")

        with self.subTest("evidence_hash_mismatch"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
            )
            (run.results_dir / "pipeline_crash_matrix.evidence.json").write_text(json.dumps({"a": 2}))
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_evidence_hash_mismatch")

        with self.subTest("evidence_missing"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
            )
            (run.results_dir / "pipeline_crash_matrix.evidence.json").unlink()
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_evidence_missing")

        for label, text in (("empty_result_file", ""), ("malformed_result_json", '{"schema": ')):
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                run = _make_run(Path(tmp))
                _write_check_files(
                    run.results_dir, "pipeline_crash_matrix", {"a": 1},
                    run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                )
                (run.results_dir / "pipeline_crash_matrix.result.json").write_text(text)
                with self.assertRaises(errors.ToolingError) as ctx:
                    results.load_results(run)
                self.assertEqual(str(ctx.exception), "check_result_schema_invalid")

        with self.subTest("malformed_evidence_json"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
            )
            (run.results_dir / "pipeline_crash_matrix.evidence.json").write_text("")
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_evidence_malformed")

        with self.subTest("extra_key"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            raw = _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
            )
            raw["unexpected"] = True
            (run.results_dir / "pipeline_crash_matrix.result.json").write_text(json.dumps(raw))
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_result_schema_invalid")

        with self.subTest("dotted_check_id"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            evidence = {"a": 1}
            evidence_hash = "sha256:" + hashlib.sha256(results.canonical(evidence)).hexdigest()
            raw = {
                "schema": results.SCHEMA,
                "run_id": run.run_id,
                "check_id": "pipeline.settle.recovery",
                "status": "pass",
                "code_revision_hash": run.code_revision_hash,
                "package_hash": None,
                "configuration_digest": None,
                "dependency_digest": None,
                "observed_at": _iso(datetime.now(timezone.utc)),
                "evidence_hash": evidence_hash,
                "safe_blockers": [],
            }
            (run.results_dir / "pipeline.settle.recovery.result.json").write_text(json.dumps(raw))
            (run.results_dir / "pipeline.settle.recovery.evidence.json").write_text(json.dumps(evidence))
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_result_schema_invalid")

        with self.subTest("digest_missing"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash, package_hash=None,
            )
            loaded = results.load_results(run)
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, loaded, required)
            self.assertEqual(str(ctx.exception), "check_result_digest_missing:pipeline_crash_matrix")

        with self.subTest("complete_and_current_passes"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
            )
            loaded = results.load_results(run)
            results.require_current_pass_results(run, loaded, required)  # must not raise


class ResultFileNamingTests(unittest.TestCase):
    """Zaki's review of #1166, minor 2: results were keyed by the `check_id`
    inside the JSON, so a second file declaring the same id replaced the
    first without a word. Each result's file must be named for the id it
    declares, and no id may be declared twice."""

    def _run_with_result(self, tmp):
        run = _make_run(Path(tmp))
        _write_check_files(
            run.results_dir, "pipeline_crash_matrix", {"a": 1},
            run_id=run.run_id, code_revision_hash=run.code_revision_hash,
        )
        return run

    def test_a_result_named_for_another_check_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self._run_with_result(tmp)
            (run.results_dir / "pipeline_crash_matrix.result.json").rename(
                run.results_dir / "pipeline_lease_renewal.result.json"
            )
            with self.assertRaises(errors.ToolingError) as ctx:
                results.load_results(run)
            self.assertEqual(str(ctx.exception), "check_result_name_mismatch")

    def test_a_second_result_for_one_check_is_refused_whatever_its_name(self):
        # Named to sort before and after the real file, so the refusal does
        # not depend on which of the two is read first.
        for other_name in ("pipeline_aaa.result.json", "pipeline_zzz.result.json"):
            with self.subTest(other_name), tempfile.TemporaryDirectory() as tmp:
                run = self._run_with_result(tmp)
                shutil.copyfile(
                    run.results_dir / "pipeline_crash_matrix.result.json", run.results_dir / other_name
                )
                with self.assertRaises(errors.ToolingError) as ctx:
                    results.load_results(run)
                self.assertEqual(str(ctx.exception), "check_result_duplicate")

    def test_a_result_named_for_its_own_check_loads(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self._run_with_result(tmp)
            self.assertEqual(set(results.load_results(run)), {"pipeline_crash_matrix"})


class EvidenceValidatorTests(unittest.TestCase):
    def test_evidence_validator_refuses_secret_like_values(self):
        refused = (
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_abcdefghijklmnopqrstuvwxyz0123456789",
            "sk-abcdefghijklmnopqrstuvwxyz0123456789",
            "someone@example.com",
            {"token": "anything"},
            {"input": "anything"},
            {"text": "anything"},
            {"trace_text": "anything"},
            {"secret": "anything"},
            {"secret_probe": "anything"},
            {"account_id": "anything"},
            {"email": "anything"},
            "x" * 129,
            1.5,
            [1.5],
            {"nested": {"secret": "x"}},
        )
        for value in refused:
            with self.subTest(value=value):
                with self.assertRaises(errors.ToolingError):
                    results.validate_evidence(value)

        accepted = (
            "pipeline_crash_matrix",
            "sha256:" + "0" * 64,
            "2026-09-30T12:00:00Z",
            0,
            3,
            True,
            False,
            None,
            ["label_one", 2, None, True],
            {"phase": "score", "count": 3, "ok": True, "hash": "sha256:" + "a" * 64},
        )
        for value in accepted:
            with self.subTest(value=value):
                results.validate_evidence(value)  # must not raise


class EnvironmentDatabaseNamingTests(unittest.TestCase):
    def test_environment_names_are_valid_test_databases(self):
        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            return (0, "0") if capture else (0, None)

        import re

        valid_name = re.compile(r"^[a-z0-9_]+$")
        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with environment.Environment(
                    run, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                ) as env:
                    for label in ("alpha_step", "beta_step", "gamma_step"):
                        scenario = env.scenario(label)
                        self.assertTrue(scenario.runtime_database.startswith("admission_test_"))
                        self.assertTrue(scenario.upgrade_database.startswith("pipeline_test_"))
                        for name in (scenario.runtime_database, scenario.upgrade_database):
                            self.assertLessEqual(len(name), 63)
                            self.assertRegex(name, valid_name)
                        self.assertIn("127.0.0.1", scenario.runtime_url)
                        self.assertIn("127.0.0.1", scenario.upgrade_url)
                        self.assertIn("127.0.0.1", scenario.login_resolver_url)
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class AdminUrlLockTests(unittest.TestCase):
    def test_admin_url_mode_holds_the_lock_database(self):
        calls = []

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append((list(command), input_text))
            return (0, "0") if capture else (0, None)

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with environment.Environment(
                    run, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                ):
                    pass
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

        creates = [text for _, text in calls if text and "CREATE DATABASE pipeline_tooling_lock" in text]
        drops = [text for _, text in calls if text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in text]
        self.assertEqual(len(creates), 1)
        self.assertEqual(len(drops), 1)

        busy_calls = []

        def fake_invoke_busy(command, *, env, capture=False, input_text=None, log_path=None):
            busy_calls.append((list(command), input_text))
            if input_text and "CREATE DATABASE pipeline_tooling_lock" in input_text:
                return 1, "ERROR: database exists"
            return (0, "0") if capture else (0, None)

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke_busy):
                with self.assertRaises(errors.ToolingError) as ctx:
                    with environment.Environment(
                        run, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                    ):
                        pass
            self.assertEqual(str(ctx.exception), "pipeline_tooling_server_busy")
            # A busy server means someone else's lock database already
            # exists: entering must never attempt to drop it.
            busy_drops = [
                text for _, text in busy_calls if text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in text
            ]
            self.assertEqual(busy_drops, [])
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class EnterFailureTeardownTests(unittest.TestCase):
    """I1: `__enter__` must tear down whatever it already created before
    re-raising, because Python never calls `__exit__` when `__enter__`
    itself raises."""

    def test_container_readiness_failure_removes_the_container(self):
        calls = []

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append(list(command))
            if command[:2] == ["docker", "run"]:
                return 0, "deadbeefcontainerid\n"
            if "pg_isready" in command:
                # Never ready: `_enter_container_mode` exhausts its 60
                # tries and raises before the port is ever read.
                return 2, "no response\n"
            if command[:2] == ["docker", "rm"]:
                return 0, "removed\n"
            if command[:3] == ["docker", "ps", "-a"]:
                return 0, ""
            return 0, ""

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke), mock.patch.object(
                environment.time, "sleep", return_value=None
            ):
                with self.assertRaises(errors.ToolingError) as ctx:
                    with environment.Environment(run):
                        pass
            self.assertEqual(str(ctx.exception), "pipeline_tooling_container_not_ready")
            rm_calls = [c for c in calls if c[:2] == ["docker", "rm"]]
            self.assertEqual(len(rm_calls), 1)
            self.assertIn(f"tc-pipeline-{run.run_id}", rm_calls[0])
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

    def test_a_failed_docker_run_removes_the_container_it_created(self):
        """Final review M2: `docker run` can create the container and then
        fail to start it (a port bind failure); teardown removes it by name,
        and a removal that leaves it behind sets `cleanup_failed`."""
        for removal_fails in (False, True):
            with self.subTest(removal_fails=removal_fails):
                calls = []

                def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
                    calls.append(list(command))
                    if command[:2] == ["docker", "run"]:
                        return 125, "port is already allocated\n"
                    if command[:3] == ["docker", "ps", "-a"]:
                        return 0, "deadbeefcontainerid\n" if removal_fails else ""
                    return 0, ""

                run = _scratch_run()
                try:
                    with mock.patch.object(environment, "_invoke", fake_invoke):
                        with self.assertRaises(errors.ToolingError) as ctx:
                            with environment.Environment(run):
                                pass
                    self.assertEqual(str(ctx.exception), "pipeline_tooling_container_start_failed")
                    self.assertIn(["docker", "rm", "-f", f"tc-pipeline-{run.run_id}"], calls)
                    self.assertIs(run.cleanup_failed, removal_fails)
                finally:
                    shutil.rmtree(run.run_dir, ignore_errors=True)

    def test_a_teardown_exception_never_replaces_the_primary_failure(self):
        """Final review M3: every teardown step runs, and one that raises
        (the docker binary gone) sets `cleanup_failed` instead of replacing
        the failure that ended the body."""
        calls = []

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append((list(command), input_text))
            if input_text and "DROP DATABASE IF EXISTS \"admission_test_" in input_text:
                raise OSError("psql vanished")
            if input_text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in input_text:
                raise OSError("psql vanished")
            return (0, "") if capture else (0, None)

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with self.assertRaises(errors.ToolingError) as ctx:
                    with environment.Environment(
                        run, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                    ) as env:
                        env.scenario("teardown_probe")
                        raise errors.ToolingError("primary_failure")
            self.assertEqual(str(ctx.exception), "primary_failure")
            self.assertTrue(run.cleanup_failed)
            lock_drops = [
                text for _, text in calls if text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in text
            ]
            self.assertEqual(len(lock_drops), 1, "the lock drop still runs after the scenario drop raised")
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

        # The same in container mode, through `main`: the primary label is
        # what the terminal shows, and the exit code is the primary's.
        def fake_container_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            if command[:2] == ["docker", "rm"]:
                raise OSError("docker vanished")
            if command[:2] == ["docker", "port"]:
                return 0, "127.0.0.1:49153\n"
            return (0, "") if capture else (0, None)

        def handler(args, r):
            with environment.Environment(r):
                raise errors.ToolingError("primary_failure")

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_container_invoke), mock.patch.object(
                pipeline, "Run"
            ) as mock_run_cls, mock.patch.object(
                pipeline, "parse_args", return_value=argparse.Namespace(handler=handler)
            ):
                mock_run_cls.create.return_value = run
                stderr = io.StringIO()
                with contextlib.redirect_stderr(stderr):
                    code = pipeline.main([])
            self.assertEqual(code, 1)
            self.assertEqual(
                stderr.getvalue().splitlines(),
                ["PipelineFailure: primary_failure", "PipelineFailure: cleanup_failed"],
            )
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

    def test_admin_url_role_sql_failure_drops_the_lock_database(self):
        calls = []
        fake_output = "ERROR: permission denied\n"

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append((list(command), input_text))
            if input_text and "CREATE DATABASE pipeline_tooling_lock" in input_text:
                return 0, ""
            if input_text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in input_text:
                return 0, ""
            if input_text and "CREATE ROLE trace_login_resolver" in input_text:
                # The step after the lock database is acquired: login-
                # resolver role creation fails.
                return 1, fake_output
            return 0, ""

        run = _scratch_run()
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with self.assertRaises(errors.StepFailed) as ctx:
                    with environment.Environment(
                        run, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                    ):
                        pass
            failure = ctx.exception
            self.assertEqual(failure.step, "environment_setup")
            self.assertEqual(failure.exit_code, 1)
            self.assertEqual(failure.log_path.read_text(), fake_output)
            drops = [
                text for _, text in calls if text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in text
            ]
            self.assertEqual(len(drops), 1)
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class SetupQueryFailureLoggingTests(unittest.TestCase):
    """R2: a failed setup query (one `Environment` runs on its own, not a
    `cargo`/`python` step) must leave the same evidence a `StepFailed` step
    does -- a protected log with the real output, and a terminal line that
    names the log path but carries none of that output."""

    def test_scenario_creation_sql_failure_logs_output_and_reports_only_the_path(self):
        fake_output = (
            'psql: error: connection to server on socket "/var/run/postgresql/.s.PGSQL.5432" '
            "failed: FATAL:  the database system is shutting down\n"
            "sk-should-never-reach-the-terminal\n"
        )

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            if input_text and "CREATE DATABASE admission_test_" in input_text:
                return 2, fake_output
            return (0, "0") if capture else (0, None)

        run = _scratch_run()

        def handler(args, r):
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with environment.Environment(
                    r, postgres_admin_url="postgres://trace@127.0.0.1:55431/postgres"
                ) as env:
                    env.scenario("postgres_probe_step")

        try:
            with mock.patch.object(pipeline, "Run") as mock_run_cls, mock.patch.object(
                pipeline, "parse_args", return_value=argparse.Namespace(handler=handler)
            ):
                mock_run_cls.create.return_value = run
                stderr = io.StringIO()
                with contextlib.redirect_stderr(stderr):
                    code = pipeline.main([])
                output = stderr.getvalue()

            self.assertEqual(code, 2)
            lines = [line for line in output.splitlines() if line]
            self.assertEqual(len(lines), 1)
            self.assertIn("postgres_probe_step", lines[0])
            self.assertIn("exit=2", lines[0])
            self.assertIn("log=", lines[0])
            self.assertNotIn(fake_output.strip(), output)
            self.assertNotIn("sk-should-never-reach-the-terminal", output)

            log_path = run.run_dir / "logs" / "postgres_probe_step.log"
            self.assertTrue(log_path.is_file())
            self.assertEqual(log_path.read_text(), fake_output)
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class MainExitCodeTests(unittest.TestCase):
    def test_cleanup_failure_fails_a_passing_run_and_keeps_an_earlier_failure(self):
        run = _scratch_run()
        quiet = io.StringIO()
        try:
            with self.subTest("success_and_clean_cleanup_returns_zero"):
                def handler_ok(args, r):
                    pass

                run.cleanup_failed = False
                with mock.patch.object(pipeline, "Run") as mock_run_cls, mock.patch.object(
                    pipeline, "parse_args", return_value=argparse.Namespace(handler=handler_ok)
                ):
                    mock_run_cls.create.return_value = run
                    with contextlib.redirect_stderr(quiet):
                        self.assertEqual(pipeline.main([]), 0)

            with self.subTest("success_with_failed_cleanup_returns_one"):
                def handler_cleanup_fails(args, r):
                    r.cleanup_failed = True

                run.cleanup_failed = False
                with mock.patch.object(pipeline, "Run") as mock_run_cls, mock.patch.object(
                    pipeline, "parse_args", return_value=argparse.Namespace(handler=handler_cleanup_fails)
                ):
                    mock_run_cls.create.return_value = run
                    with contextlib.redirect_stderr(quiet):
                        self.assertEqual(pipeline.main([]), 1)

            with self.subTest("step_failure_and_failed_cleanup_keeps_the_step_exit_code"):
                run.cleanup_failed = False
                log_path = run.log_path("faked_step")

                def handler_fails(args, r):
                    r.cleanup_failed = True
                    raise pipeline.StepFailed("faked_step", 7, log_path)

                with mock.patch.object(pipeline, "Run") as mock_run_cls, mock.patch.object(
                    pipeline, "parse_args", return_value=argparse.Namespace(handler=handler_fails)
                ):
                    mock_run_cls.create.return_value = run
                    with contextlib.redirect_stderr(quiet):
                        self.assertEqual(pipeline.main([]), 7)
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


class FailureOutputTests(unittest.TestCase):
    def test_failure_output_is_label_only(self):
        run = _scratch_run()
        secret_url = "https://example.invalid/super-secret-path?token=1"
        secret_token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789"

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            if log_path is not None:
                Path(log_path).write_text(f"{secret_url}\n{secret_token}\n")
            return (9, "") if capture else (9, None)

        def handler(args, r):
            environment.run_child(
                r, "faked_step", ["fake-command", secret_url, secret_token], {"SOME_ENV": secret_token}
            )

        try:
            with mock.patch.object(environment, "_invoke", fake_invoke), mock.patch.object(
                pipeline, "Run"
            ) as mock_run_cls, mock.patch.object(
                pipeline, "parse_args", return_value=argparse.Namespace(handler=handler)
            ):
                mock_run_cls.create.return_value = run
                stderr = io.StringIO()
                with contextlib.redirect_stderr(stderr):
                    code = pipeline.main([])
            output = stderr.getvalue()
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

        self.assertEqual(code, 9)
        lines = [line for line in output.splitlines() if line]
        self.assertEqual(len(lines), 1)
        self.assertIn("faked_step", lines[0])
        self.assertIn("exit=9", lines[0])
        self.assertIn("log=", lines[0])
        self.assertNotIn(secret_url, output)
        self.assertNotIn(secret_token, output)
        self.assertNotIn("fake-command", output)


class HfCorpusTests(unittest.TestCase):
    """Task 4: `pipeline_tooling.corpus` -- `load_direct_corpus`, `load_pin`,
    `export_hf_corpus`."""

    def _base_fixture(self):
        return {
            "label": "hf_bootstrap_0000",
            "trace_id": str(uuid.uuid4()),
            "submission_id": str(uuid.uuid4()),
            "secret_probe": "qualification_probe_bootstrap_0000",
        }

    def _write_corpus(self, tmp, fixtures, schema=corpus.CORPUS_SCHEMA):
        path = Path(tmp) / "corpus.json"
        path.write_text(json.dumps({"schema": schema, "fixtures": fixtures}))
        return path

    def test_corpus_validation_refuses_changed_bytes_order_and_duplicates(self):
        base = self._base_fixture()

        with self.subTest("unsupported_schema"), tempfile.TemporaryDirectory() as tmp:
            path = self._write_corpus(tmp, [dict(base)], schema="wrong.schema.v1")
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "unsupported_corpus_schema")

        with self.subTest("empty_fixtures"), tempfile.TemporaryDirectory() as tmp:
            path = self._write_corpus(tmp, [])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "empty_corpus")

        with self.subTest("duplicate_label"), tempfile.TemporaryDirectory() as tmp:
            first = dict(base)
            second = dict(base, trace_id=str(uuid.uuid4()), submission_id=str(uuid.uuid4()))
            path = self._write_corpus(tmp, [first, second])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "duplicate_corpus_identity")

        with self.subTest("duplicate_trace_id"), tempfile.TemporaryDirectory() as tmp:
            first = dict(base, label="hf_bootstrap_0000")
            second = dict(base, label="hf_bootstrap_0001", submission_id=str(uuid.uuid4()))
            path = self._write_corpus(tmp, [first, second])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "duplicate_corpus_identity")

        with self.subTest("duplicate_submission_id"), tempfile.TemporaryDirectory() as tmp:
            first = dict(base, label="hf_bootstrap_0000")
            second = dict(base, label="hf_bootstrap_0001", trace_id=str(uuid.uuid4()))
            path = self._write_corpus(tmp, [first, second])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "duplicate_corpus_identity")

        with self.subTest("missing_trace_id"), tempfile.TemporaryDirectory() as tmp:
            bad = dict(base)
            del bad["trace_id"]
            path = self._write_corpus(tmp, [bad])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "missing_corpus_identity")

        with self.subTest("missing_submission_id"), tempfile.TemporaryDirectory() as tmp:
            bad = dict(base)
            del bad["submission_id"]
            path = self._write_corpus(tmp, [bad])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "missing_corpus_identity")

        with self.subTest("unsafe_label"), tempfile.TemporaryDirectory() as tmp:
            bad = dict(base, label="Not Safe!")
            path = self._write_corpus(tmp, [bad])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "unsafe_fixture_label")

        with self.subTest("empty_secret_probe"), tempfile.TemporaryDirectory() as tmp:
            bad = dict(base, secret_probe="")
            path = self._write_corpus(tmp, [bad])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path)
            self.assertEqual(str(ctx.exception), "empty_secret_probe")

        with self.subTest("digest_mismatch"), tempfile.TemporaryDirectory() as tmp:
            path = self._write_corpus(tmp, [dict(base)])
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_direct_corpus(path, expected_digest=_fake_hash("not-this-corpus"))
            self.assertEqual(str(ctx.exception), "corpus_digest_mismatch")

        with self.subTest("digest_match_and_shape_ok"), tempfile.TemporaryDirectory() as tmp:
            path = self._write_corpus(tmp, [dict(base)])
            actual_digest = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
            loaded, digest_value = corpus.load_direct_corpus(path, expected_digest=actual_digest)
            self.assertEqual(digest_value, actual_digest)
            self.assertEqual(loaded["fixtures"][0]["label"], base["label"])

        valid_pin = {
            "schema": corpus.PIN_SCHEMA,
            "repository": "jedisct1/security-audits",
            "revision": "deadbeef",
            "split": "train",
            "translator": "swival",
            "bootstrap_count": 1,
            "holdout_count": 1,
            "min_words": 1,
            "max_words": 2000,
            "expected_instrument_count": 1,
            "source_digest": _fake_hash("source"),
            "order_digest": _fake_hash("order"),
        }

        with self.subTest("pin_wrong_schema"), tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pin.json"
            path.write_text(json.dumps(dict(valid_pin, schema="wrong.schema.v1")))
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_pin(path)
            self.assertEqual(str(ctx.exception), "unsupported_pin_schema")

        with self.subTest("pin_missing_field"), tempfile.TemporaryDirectory() as tmp:
            incomplete = dict(valid_pin)
            del incomplete["order_digest"]
            path = Path(tmp) / "pin.json"
            path.write_text(json.dumps(incomplete))
            with self.assertRaises(errors.ToolingError) as ctx:
                corpus.load_pin(path)
            self.assertEqual(str(ctx.exception), "pin_missing_field")

        with self.subTest("pin_valid"), tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pin.json"
            path.write_text(json.dumps(valid_pin))
            loaded = corpus.load_pin(path)
            self.assertEqual(loaded["repository"], valid_pin["repository"])

        def _fake_export_invoke(manifest, calls=None):
            def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
                if calls is not None:
                    calls.append(list(command))
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "bootstrap-corpus.json").write_text("{}")
                (output_dir / "holdout-corpus.json").write_text("{}")
                (output_dir / "source-manifest.json").write_text(json.dumps(manifest))
                return (0, "") if capture else (0, None)

            return fake_invoke

        def _base_manifest(pin):
            return {
                "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
                "source_digest": pin["source_digest"],
                "order_digest": pin["order_digest"],
                "configuration_digest": _fake_hash("configuration"),
                "bootstrap_corpus_digest": _fake_hash("bootstrap"),
                "holdout_corpus_digest": _fake_hash("holdout"),
                "contains_raw_trace_text": False,
                "contains_contributor_identity": False,
            }

        for field in ("source_digest", "order_digest", "bootstrap_corpus_digest", "holdout_corpus_digest"):
            with self.subTest(f"export_manifest_mismatch_{field}"):
                run = _scratch_run()
                pin = dict(
                    valid_pin,
                    bootstrap_corpus_digest=_fake_hash("bootstrap"),
                    holdout_corpus_digest=_fake_hash("holdout"),
                )
                manifest = _base_manifest(pin)
                manifest[field] = _fake_hash("a-different-value")
                try:
                    with tempfile.TemporaryDirectory() as tmp:
                        pin_path = Path(tmp) / "pin.json"
                        pin_path.write_text(json.dumps(pin))
                        with mock.patch.object(environment, "_invoke", _fake_export_invoke(manifest)):
                            with self.assertRaises(errors.ToolingError) as ctx:
                                corpus.export_hf_corpus(run, pin_path, {})
                        self.assertEqual(str(ctx.exception), f"hf_{field}_mismatch")
                finally:
                    shutil.rmtree(run.run_dir, ignore_errors=True)

        with self.subTest("export_refuses_raw_trace_text"):
            run = _scratch_run()
            pin = dict(
                valid_pin,
                bootstrap_corpus_digest=_fake_hash("bootstrap"),
                holdout_corpus_digest=_fake_hash("holdout"),
            )
            manifest = _base_manifest(pin)
            manifest["bootstrap_corpus_digest"] = pin["bootstrap_corpus_digest"]
            manifest["holdout_corpus_digest"] = pin["holdout_corpus_digest"]
            manifest["contains_raw_trace_text"] = True
            try:
                with tempfile.TemporaryDirectory() as tmp:
                    pin_path = Path(tmp) / "pin.json"
                    pin_path.write_text(json.dumps(pin))
                    with mock.patch.object(environment, "_invoke", _fake_export_invoke(manifest)):
                        with self.assertRaises(errors.ToolingError) as ctx:
                            corpus.export_hf_corpus(run, pin_path, {})
                    self.assertEqual(str(ctx.exception), "hf_manifest_contains_raw_trace_text")
            finally:
                shutil.rmtree(run.run_dir, ignore_errors=True)

        with self.subTest("export_success_returns_bootstrap_then_holdout"):
            run = _scratch_run()
            pin = dict(
                valid_pin,
                bootstrap_corpus_digest=_fake_hash("bootstrap"),
                holdout_corpus_digest=_fake_hash("holdout"),
            )
            manifest = _base_manifest(pin)
            manifest["bootstrap_corpus_digest"] = pin["bootstrap_corpus_digest"]
            manifest["holdout_corpus_digest"] = pin["holdout_corpus_digest"]
            try:
                with tempfile.TemporaryDirectory() as tmp:
                    pin_path = Path(tmp) / "pin.json"
                    pin_path.write_text(json.dumps(pin))
                    calls = []
                    with mock.patch.object(environment, "_invoke", _fake_export_invoke(manifest, calls)):
                        outputs = corpus.export_hf_corpus(run, pin_path, {})
                    self.assertEqual(len(outputs), 2)
                    self.assertEqual(outputs[0].name, "bootstrap-corpus.json")
                    self.assertEqual(outputs[1].name, "holdout-corpus.json")
                    for path in outputs:
                        self.assertTrue(path.is_file())
                    self.assertIn("--expected-source-digest", calls[0])
                    self.assertIn("--expected-order-digest", calls[0])
                    self.assertNotIn("--local-jsonl-dir", calls[0])
                    self.assertIn("--cache-dir", calls[0])
                    self.assertEqual(
                        calls[0][calls[0].index("--cache-dir") + 1], str(corpus.HF_CACHE_DIR)
                    )
            finally:
                shutil.rmtree(run.run_dir, ignore_errors=True)

        with self.subTest("export_local_dir_adds_flag_and_keeps_expected_digests"):
            run = _scratch_run()
            pin = dict(
                valid_pin,
                bootstrap_corpus_digest=_fake_hash("bootstrap"),
                holdout_corpus_digest=_fake_hash("holdout"),
            )
            manifest = _base_manifest(pin)
            manifest["bootstrap_corpus_digest"] = pin["bootstrap_corpus_digest"]
            manifest["holdout_corpus_digest"] = pin["holdout_corpus_digest"]
            try:
                with tempfile.TemporaryDirectory() as tmp:
                    pin_path = Path(tmp) / "pin.json"
                    pin_path.write_text(json.dumps(pin))
                    calls = []
                    with mock.patch.object(environment, "_invoke", _fake_export_invoke(manifest, calls)):
                        corpus.export_hf_corpus(run, pin_path, {}, local_dir="fixtures/pipeline-hf-jsonl")
                    self.assertIn("--local-jsonl-dir", calls[0])
                    self.assertIn("fixtures/pipeline-hf-jsonl", calls[0])
                    self.assertIn("--expected-source-digest", calls[0])
                    self.assertIn("--expected-order-digest", calls[0])
                    self.assertIn("--cache-dir", calls[0])
                    self.assertEqual(
                        calls[0][calls[0].index("--cache-dir") + 1], str(corpus.HF_CACHE_DIR)
                    )
            finally:
                shutil.rmtree(run.run_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Task 9: `pipeline.py run` and `pipeline.py package`.
# ---------------------------------------------------------------------------

_ADMIN_URL = "postgres://trace@127.0.0.1:55431/postgres"
_HARNESS = "tests::pipeline_corpus_pg_tests::pipeline_corpus_run"
_PACKAGE_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_package_write"
_INGEST_ARGS = ("-p", "trace-commons-server", "--bin", "trace-commons-ingest")


def _digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _direct_corpus(labels, prefix="1"):
    return {
        "schema": corpus.CORPUS_SCHEMA,
        "fixtures": [
            {
                "label": label,
                "trace_id": str(uuid.uuid5(uuid.NAMESPACE_URL, f"trace:{prefix}:{label}")),
                "submission_id": str(uuid.uuid5(uuid.NAMESPACE_URL, f"submission:{prefix}:{label}")),
                "created_at": "2026-09-11T12:00:00Z",
                "input": "fixture input text",
                "secret_probe": f"probe_{label}",
            }
            for label in labels
        ],
    }


def _fixture_report(label, **overrides):
    """One fixture of a `trace_commons.pipeline_corpus_report.v1` report, in
    the shape `pipeline_corpus_pg_tests` writes: an admitted, complete run
    unless `overrides` says otherwise."""
    item = {
        "label": label,
        "run_id_hash": _fake_hash(f"run:{label}"),
        "submission_id_hash": _fake_hash(f"submission:{label}"),
        "request_content_hash": _fake_hash(f"request:{label}"),
        "state": "complete",
        "admission_decision": "admit",
        "expected_admission_decision": "admit",
        "admission_reason": None,
        "responsible_phase": "settle",
        "reason_label": None,
        "phase_count": 4,
        "expected_outcome_count": 4,
        "phases": [
            {
                "phase": phase,
                "outcome_id_hash": _fake_hash(f"outcome:{label}:{phase}"),
                "outcome_schema_id": f"trace_commons.{phase}_outcome",
                "outcome_schema_version": 1,
                "decision_hash": _fake_hash(f"decision:{label}:{phase}"),
                "evidence_hash": _fake_hash(f"evidence:{label}:{phase}"),
                "evaluation_hash": _fake_hash(f"evaluation:{label}:{phase}"),
            }
            for phase in corpus.PHASES
        ],
        "index_command_hash": _fake_hash(f"index:{label}"),
        "index_write_state": "complete",
        "index_invalidation_state": "none",
        "credit_write_state": "complete",
        "payout_state": "none",
        "instruments": [
            {
                "instrument_id": "storage_rebate",
                "atomic_units": "5",
                "operation_state": "complete",
                "internal_settlement_state": "not_applicable",
                "payout_rail": "none",
                "payout_state": "disabled",
            }
        ],
        "replay_same_run": True,
        "changed_content_refused": True,
        "tenant_isolation": True,
        "consent_state": "allowed",
        "expected_consent_state": "allowed",
        "privacy_state": "low",
        "expected_privacy_state": "low",
        "scoring_state": "complete",
        "expected_scoring_state": "complete",
        "settlement_state": "complete",
        "expected_settlement_state": "complete",
        "instrument_count": 1,
        "expected_instrument_count": 1,
        "instrument_states": {"storage_rebate": "not_applicable"},
        "expected_instrument_states": {"storage_rebate": "not_applicable"},
        "mismatches": [],
    }
    item.update(overrides)
    return item


def _policy_manifest():
    def policy(phase):
        return {
            "policy_id": f"trace_commons.{phase}.minimal",
            "implementation_id": f"trace_commons.{phase}.minimal.v1",
            "configuration_hash": _fake_hash("configuration"),
            "data_artifact_hashes": [],
            "projection_ids": [],
        }

    return {
        "format_version": 1,
        "admission": policy("admission"),
        "review": policy("review"),
        "score": policy("score"),
        "settle": policy("settle"),
        "instruments": {},
    }


def _corpus_report(check_id, partitions):
    """`partitions` is a list of `(name, corpus_digest, [fixture reports])`."""
    manifest = _policy_manifest()
    bundle_id = _fake_hash("bundle")
    sections = []
    for name, corpus_digest, fixtures in partitions:
        sections.append(
            {
                "partition": name,
                "bundle_id": bundle_id,
                "corpus_digest": corpus_digest,
                "fixture_order": [item["label"] for item in fixtures],
                "expected_fixture_count": len(fixtures),
                "completed_fixture_count": sum(item["state"] in ("complete", "rejected") for item in fixtures),
                "failure_count": sum(bool(item["mismatches"]) for item in fixtures),
                "fixtures": fixtures,
            }
        )
    every = [item for section in sections for item in section["fixtures"]]
    report = {
        "schema": corpus.REPORT_SCHEMA,
        "scope": "local_test",
        "production_ready": False,
        "external_payout_enabled": False,
        "safe_blockers": list(corpus.LOCAL_BLOCKERS),
        "check_id": check_id,
        "bundle_id": bundle_id,
        "package_hash": _fake_hash("package"),
        "configuration_digest": _digest(
            results.canonical({phase: manifest[phase]["configuration_hash"] for phase in corpus.PHASES})
        ),
        "dependency_digest": _fake_hash("dependency"),
        "policy_manifest": manifest,
        "fixture_count": len(every),
        "completed_fixture_count": sum(section["completed_fixture_count"] for section in sections),
        "failure_count": sum(section["failure_count"] for section in sections),
        "replay_same_run_count": sum(item["replay_same_run"] for item in every),
        "changed_content_refused_count": sum(item["changed_content_refused"] for item in every),
        "tenant_isolation": all(item["tenant_isolation"] for item in every),
        "partitions": sections,
    }
    report["report_digest"] = _digest(results.canonical(report))
    return report


def _resigned(report):
    """`report` with its `report_digest` recomputed over its other fields."""
    unsigned = {key: value for key, value in report.items() if key != "report_digest"}
    return {**unsigned, "report_digest": _digest(results.canonical(unsigned))}


def _write_harness_outputs(env, fixtures_by_partition=None, emit=True):
    """What `pipeline_corpus_run` leaves behind: the report at
    `TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH` and, when every fixture
    passed, one check result and its evidence in the result directory."""
    paths = [("corpus", env["TRACE_COMMONS_PIPELINE_CORPUS_PATH"])]
    if "TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH" in env:
        paths = [
            ("bootstrap", env["TRACE_COMMONS_PIPELINE_CORPUS_PATH"]),
            ("holdout", env["TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH"]),
        ]
    partitions = []
    for name, path in paths:
        data = Path(path).read_bytes()
        labels = [item["label"] for item in json.loads(data)["fixtures"]]
        fixtures = (fixtures_by_partition or {}).get(name) or [_fixture_report(label) for label in labels]
        partitions.append((name, _digest(data), fixtures))
    check_id = env["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"]
    report = _corpus_report(check_id, partitions)
    report_bytes = results.canonical(report) + b"\n"
    Path(env["TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH"]).write_bytes(report_bytes)
    if not emit:
        return report
    evidence = {
        "fixtures": report["fixture_count"],
        "completed": report["completed_fixture_count"],
        "replay_same_run": report["replay_same_run_count"],
        "changed_content_refused": report["changed_content_refused_count"],
        "tenant_isolation": report["tenant_isolation"],
        "report_hash": _digest(report_bytes),
    }
    result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
    raw = {
        "schema": results.SCHEMA,
        "run_id": env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"],
        "check_id": check_id,
        "status": "pass",
        "code_revision_hash": env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"],
        "package_hash": report["package_hash"],
        "configuration_digest": report["configuration_digest"],
        "dependency_digest": report["dependency_digest"],
        "observed_at": _iso(datetime.now(timezone.utc)),
        "evidence_hash": _digest(results.canonical(evidence)),
        "safe_blockers": [],
    }
    (result_dir / f"{check_id}.result.json").write_text(json.dumps(raw))
    (result_dir / f"{check_id}.evidence.json").write_text(json.dumps(evidence))
    return report


def _environment_invoke(calls):
    """Stands in for Docker and psql: every call succeeds, and a committed-
    transaction query answers 42 (above the xact guard's floor)."""

    def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
        calls.append(("invoke", list(command), input_text))
        return (0, "42") if capture else (0, None)

    return fake_invoke


class _CorpusRunCase(unittest.TestCase):
    def setUp(self):
        self.run = _scratch_run("corpus")
        self.tmp = Path(tempfile.mkdtemp())
        self.calls = []
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        # The tree hash a command recomputes at its end
        # (`Run.require_code_revision_unchanged`): the run's own unless a
        # test sets `tree_edited`. `tree_hash_calls` counts the recomputes.
        self.tree_edited = False
        self.tree_hash_calls = 0

    def tearDown(self):
        shutil.rmtree(self.run.run_dir, ignore_errors=True)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _tree_hash(self):
        self.tree_hash_calls += 1
        return _fake_hash("edited-tree") if self.tree_edited else self.run.code_revision_hash

    def _main(self, argv, cargo=None, export=None):
        patches = [
            mock.patch.object(environment, "_invoke", _environment_invoke(self.calls)),
            mock.patch.object(pipeline, "LOCAL_DIR", self.tmp / "local"),
            mock.patch.object(pipeline, "Run"),
            mock.patch.object(environment, "_code_revision_hash", self._tree_hash),
        ]
        if cargo is not None:
            patches.append(mock.patch.object(pipeline, "cargo_test", cargo))
        if export is not None:
            patches.append(mock.patch.object(pipeline, "export_hf_corpus", export))
        with contextlib.ExitStack() as stack:
            mocks = [stack.enter_context(patch) for patch in patches]
            mocks[2].create.return_value = self.run
            stack.enter_context(contextlib.redirect_stdout(self.stdout))
            stack.enter_context(contextlib.redirect_stderr(self.stderr))
            return pipeline.main(argv)

    def _fake_cargo(self, **harness):
        def fake_cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            if test_filter == _HARNESS:
                _write_harness_outputs(env, **harness)

        return fake_cargo_test

    def _cargo_calls(self):
        return [call for call in self.calls if call[0] == "cargo"]


class CorpusRunTests(_CorpusRunCase):
    def test_run_passes_only_the_expected_variables(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture", "beta_fixture"])))

        code = self._main(
            ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL],
            cargo=self._fake_cargo(),
        )

        self.assertEqual(code, 0, self.stderr.getvalue())
        cargo_calls = self._cargo_calls()
        self.assertEqual(len(cargo_calls), 1)
        _, step, cargo_args, test_filter, env, exact, ignored = cargo_calls[0]
        self.assertEqual(step, "corpus_run")
        self.assertEqual(cargo_args, _INGEST_ARGS)
        self.assertEqual(test_filter, _HARNESS)
        self.assertTrue(exact)
        self.assertTrue(ignored)
        self.assertEqual(
            {key for key in env if key.startswith("TRACE_COMMONS_")},
            {
                "TRACE_COMMONS_PG_TEST_DATABASE_URL",
                "TRACE_COMMONS_PIPELINE_CORPUS_PATH",
                "TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID",
                "TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE",
                "TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH",
                "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT",
                "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX",
                "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR",
                "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID",
                "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH",
            },
        )
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"], "pipeline_http_corpus_minimal")
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE"], "minimal")
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_PATH"], str(corpus_path.resolve()))
        self.assertIn("/admission_test_", env["TRACE_COMMONS_PG_TEST_DATABASE_URL"])
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"], self.run.run_id)
        master_key = env["TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX"]
        self.assertRegex(master_key, r"^[0-9a-f]{64}$")

        output = self.stdout.getvalue() + self.stderr.getvalue()
        self.assertNotIn(master_key, output)
        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(lines, [f"PipelineRunOK: bundle={_fake_hash('bundle')} fixtures=2"])
        local_report = self.tmp / "local" / "pipeline-minimal-corpus-report.json"
        self.assertTrue(local_report.is_file())
        self.assertTrue(local_report.with_suffix(".md").is_file())
        self.assertFalse((self.tmp / "local" / "pipeline-lab-catalog.json").exists())

    def test_run_refuses_package_and_bundle_together(self):
        package = self.tmp / "package.json"
        key = self.tmp / "key.json"
        cases = (
            (["--bundle", "minimal", "--package", str(package), "--trusted-key", str(key)],
             "corpus_package_and_bundle_conflict"),
            (["--package", str(package)], "corpus_package_and_key_required"),
            (["--trusted-key", str(key)], "corpus_package_and_key_required"),
            ([], "corpus_bundle_or_package_required"),
        )
        for extra, label in cases:
            with self.subTest(label=label, extra=extra):
                self.calls.clear()
                self.stderr = io.StringIO()
                code = self._main(["run", *extra], cargo=self._fake_cargo())
                self.assertEqual(code, 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")
                self.assertEqual(self.calls, [], "refused before any environment or cargo call")

    def test_hf_pin_runs_bootstrap_then_holdout(self):
        pin_path = self.tmp / "pin.json"
        pin_path.write_text(
            json.dumps(
                {
                    "schema": corpus.PIN_SCHEMA,
                    "repository": "jedisct1/security-audits",
                    "revision": "deadbeef",
                    "split": "train",
                    "translator": "swival",
                    "bootstrap_count": 1,
                    "holdout_count": 1,
                    "min_words": 1,
                    "max_words": 2000,
                    "expected_instrument_count": 1,
                    "source_digest": _fake_hash("source"),
                    "order_digest": _fake_hash("order"),
                    "local_jsonl_dir": "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl",
                }
            )
        )

        def fake_export(run, path, env, *, local_dir=None):
            self.calls.append(("export", Path(path), local_dir))
            output = run.run_dir / "hf"
            output.mkdir(parents=True, exist_ok=True)
            bootstrap = output / "bootstrap-corpus.json"
            holdout = output / "holdout-corpus.json"
            bootstrap.write_text(json.dumps(_direct_corpus(["hf_bootstrap_0000"], prefix="b")))
            holdout.write_text(json.dumps(_direct_corpus(["hf_holdout_0000"], prefix="h")))
            return [bootstrap, holdout]

        code = self._main(
            ["run", "--bundle", "compatibility", "--corpus", str(pin_path), "--postgres-admin-url", _ADMIN_URL],
            cargo=self._fake_cargo(),
            export=fake_export,
        )

        self.assertEqual(code, 0, self.stderr.getvalue())
        order = [call[0] for call in self.calls if call[0] in ("export", "cargo")]
        self.assertEqual(order, ["export", "cargo"], "export first, then one harness run")
        export_call = next(call for call in self.calls if call[0] == "export")
        self.assertEqual(export_call[1], pin_path.resolve())
        self.assertEqual(
            Path(export_call[2]),
            environment.ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl",
        )
        _, _, _, test_filter, env, exact, ignored = self._cargo_calls()[0]
        self.assertEqual(test_filter, _HARNESS)
        self.assertTrue(exact and ignored)
        hf_dir = self.run.run_dir / "hf"
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_PATH"], str(hf_dir / "bootstrap-corpus.json"))
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH"], str(hf_dir / "holdout-corpus.json"))
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"], "pipeline_http_corpus_hf_local")
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE"], "compatibility")
        self.assertIn("PipelineRunOK:", self.stdout.getvalue())
        self.assertIn("fixtures=2", self.stdout.getvalue())

    def test_run_keeps_the_report_when_the_harness_fails(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture", "beta_fixture"])))
        mismatched = _fixture_report(
            "beta_fixture", expected_admission_decision="reject", mismatches=["admission_decision"]
        )
        log_path = self.run.log_path("corpus_run")

        def failing_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            _write_harness_outputs(
                env,
                fixtures_by_partition={"corpus": [_fixture_report("alpha_fixture"), mismatched]},
                emit=False,
            )
            raise errors.StepFailed(step, 101, log_path)

        code = self._main(
            ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL],
            cargo=failing_cargo,
        )

        self.assertEqual(code, 101)
        self.assertIn("PipelineFailure: step_failed:corpus_run exit=101", self.stderr.getvalue())
        local_report = self.tmp / "local" / "pipeline-minimal-corpus-report.json"
        report = json.loads(local_report.read_text())
        self.assertEqual(report["failure_count"], 1)
        self.assertEqual(report["partitions"][0]["fixtures"][1]["mismatches"], ["admission_decision"])
        self.assertIn("admission_decision", local_report.with_suffix(".md").read_text())
        self.assertNotIn("PipelineRunOK", self.stdout.getvalue())

    def test_a_failing_harness_with_a_malformed_report_keeps_its_exit_code(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        log_path = self.run.log_path("corpus_run")
        malformed_reports = (
            b"{not json",
            b"[]",
            b'{"schema": "trace_commons.pipeline_corpus_report.v1"}',
            # Signed, so it passes the digest check and reaches the
            # partitions, where a non-object section used to raise.
            results.canonical(_resigned({**_corpus_report("pipeline_http_corpus_minimal", []), "partitions": [None]})),
        )
        for index, malformed in enumerate(malformed_reports):
            with self.subTest(index=index):
                self.stdout, self.stderr = io.StringIO(), io.StringIO()

                def failing_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
                    Path(env["TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH"]).write_bytes(malformed)
                    raise errors.StepFailed(step, 101, log_path)

                code = self._main(
                    ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL],
                    cargo=failing_cargo,
                )
                self.assertEqual(code, 101)
                lines = [line for line in self.stderr.getvalue().splitlines() if line]
                self.assertEqual(len(lines), 1, self.stderr.getvalue())
                self.assertTrue(lines[0].startswith("PipelineFailure: step_failed:corpus_run exit=101 log="))
                self.assertEqual(self.stdout.getvalue(), "")
                self.assertFalse((self.tmp / "local" / "pipeline-minimal-corpus-report.json").exists())

    def test_run_archives_only_with_archive(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        catalog_path = self.tmp / "local" / "pipeline-lab-catalog.json"
        argv = ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL]

        self.assertEqual(self._main(argv, cargo=self._fake_cargo()), 0, self.stderr.getvalue())
        self.assertFalse(catalog_path.exists(), "a routine run writes no catalog")

        shutil.rmtree(self.run.run_dir)
        self.run = _scratch_run("corpus")
        self.assertEqual(self._main([*argv, "--archive"], cargo=self._fake_cargo()), 0, self.stderr.getvalue())
        catalog_value = json.loads(catalog_path.read_text())
        self.assertEqual(catalog_value["schema"], "trace_commons.pipeline_lab_catalog.v1")
        entry = catalog_value["bundles"][0]
        self.assertEqual(entry["bundle_id"], _fake_hash("bundle"))
        record = entry["reports"][0]
        self.assertEqual(record["status"], "pass")
        self.assertTrue((catalog_path.parent / record["report"]).is_file())

    def test_run_refuses_a_report_that_does_not_match_its_evidence(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))

        def tampering_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step))
            _write_harness_outputs(env)
            report_path = Path(env["TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH"])
            report_path.write_bytes(report_path.read_bytes() + b" ")

        code = self._main(
            ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL],
            cargo=tampering_cargo,
        )
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: corpus_evidence_mismatch")


class CorpusReportValidationTests(unittest.TestCase):
    def test_corpus_report_validation_refuses_unsafe_or_inconsistent_reports(self):
        base = _corpus_report(
            "pipeline_http_corpus_minimal",
            [("corpus", _fake_hash("corpus"), [_fixture_report("alpha_fixture")])],
        )
        corpus.validate_report(base)  # must not raise

        def resign(report):
            report = dict(report)
            report.pop("report_digest")
            report["report_digest"] = _digest(results.canonical(report))
            return report

        def with_fixture(**overrides):
            report = json.loads(json.dumps(base))
            report["partitions"][0]["fixtures"][0].update(overrides)
            return resign(report)

        def without_fixture_field(field):
            report = json.loads(json.dumps(base))
            del report["partitions"][0]["fixtures"][0][field]
            return resign(report)

        withheld_instruments = [dict(_fixture_report("alpha_fixture")["instruments"][0],
                                     internal_settlement_state="withheld")]
        cases = {
            # A raw run id would be hashed by `safe_report_value`: the report
            # must already carry only its hash.
            "unsafe_report": with_fixture(reason_label=str(uuid.uuid4())),
            "unsafe_report_value": with_fixture(reason_label="free text with spaces"),
            # A static bearer credential's shape (ruling T9-7).
            "unsafe_report_value:token": with_fixture(reason_label="token-corpus-lab-admin"),
            # The leg states must be the ones the fixture's instruments show...
            "instrument_states_mismatch": with_fixture(instrument_states={"storage_rebate": "withheld"}),
            # ...and a withheld leg where a credit was expected is a failure
            # the fixture must name (ruling T9-6).
            "qualification_mismatch_not_failed:withheld": with_fixture(
                instruments=withheld_instruments, instrument_states={"storage_rebate": "withheld"}
            ),
            "corpus_report_malformed": with_fixture(instruments=[None]),
            "corpus_report_malformed:missing": without_fixture_field("state"),
            "private_report_field": with_fixture(secret_probe="x"),
            "report_digest_mismatch": dict(base, report_digest=_fake_hash("other")),
            "qualification_mismatch_not_failed": with_fixture(admission_decision="reject"),
            "invalid_report_scope": resign(dict(base, production_ready=True)),
            "unsupported_report_schema": resign(dict(base, schema="trace_commons.pipeline_corpus_report.v5")),
        }
        for case, report in cases.items():
            label = case.split(":")[0]
            with self.subTest(case=case):
                with self.assertRaises(errors.ToolingError) as ctx:
                    corpus.validate_report(report)
                self.assertEqual(str(ctx.exception), label)
        for malformed in ([], "report", 3, None, {"schema": corpus.REPORT_SCHEMA, "scope": "local_test",
                                                   "production_ready": False, "external_payout_enabled": False,
                                                   "safe_blockers": 7}):
            with self.subTest(malformed=malformed):
                with self.assertRaises(errors.ToolingError):
                    corpus.validate_report(malformed)

        failed = with_fixture(admission_decision="reject", mismatches=["admission_decision"])
        failed["partitions"][0]["failure_count"] = 1
        failed["failure_count"] = 1
        corpus.validate_report(resign(failed))  # a recorded failure is a valid report
        self.assertIn("admission_decision", corpus.markdown(resign(failed)))


class PackageCommandTests(_CorpusRunCase):
    def test_package_runs_the_package_writer_with_its_own_variables(self):
        output = self.tmp / "out" / "package.json"
        key_output = self.tmp / "out" / "trusted-key.json"

        def fake_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            Path(env["TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT"]).write_text(
                json.dumps(
                    {
                        "package": {"bundle_id": _fake_hash("bundle")},
                        "signature": {"package_hash": _fake_hash("package"), "key_id": "local_pipeline_ephemeral"},
                    }
                )
            )
            Path(env["TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT"]).write_text("{}")

        code = self._main(
            ["package", "--bundle", "minimal", "--output", str(output), "--public-key-output", str(key_output)],
            cargo=fake_cargo,
        )
        self.assertEqual(code, 0, self.stderr.getvalue())
        [(_, step, cargo_args, test_filter, env, exact, ignored)] = self._cargo_calls()
        self.assertEqual((step, cargo_args, test_filter, exact, ignored),
                         ("package_write", _INGEST_ARGS, _PACKAGE_WRITER, True, True))
        self.assertEqual(
            {key: value for key, value in env.items() if key.startswith("TRACE_COMMONS_")},
            {
                "TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE": "minimal",
                "TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT": str(output.resolve()),
                "TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT": str(key_output.resolve()),
            },
        )
        self.assertEqual(
            self.stdout.getvalue().strip(),
            f"PipelinePackageOK: bundle={_fake_hash('bundle')} package={_fake_hash('package')}",
        )

        for written in (b"{not json", b"[]", b'{"package": [], "signature": 3}'):
            with self.subTest(written=written):
                self.calls.clear()
                self.stdout, self.stderr = io.StringIO(), io.StringIO()

                def malformed_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
                    self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
                    Path(env["TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT"]).write_bytes(written)
                    Path(env["TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT"]).write_text("{}")

                code = self._main(
                    ["package", "--bundle", "minimal", "--output", str(output), "--public-key-output", str(key_output)],
                    cargo=malformed_cargo,
                )
                self.assertEqual(code, 1)
                self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: package_output_invalid")

        for extra, label in (
            (["--signing-key", str(self.tmp / "key.der")], "package_signing_key_and_key_id_required"),
            (["--key-id", "operator_key"], "package_signing_key_and_key_id_required"),
        ):
            with self.subTest(label=label, extra=extra):
                self.calls.clear()
                self.stderr = io.StringIO()
                code = self._main(
                    ["package", "--bundle", "minimal", "--output", str(output),
                     "--public-key-output", str(key_output), *extra],
                    cargo=fake_cargo,
                )
                self.assertEqual(code, 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")
                self.assertEqual(self._cargo_calls(), [])


# ---------------------------------------------------------------------------
# Task 10: `pipeline.py restore-drill`.
# ---------------------------------------------------------------------------

_RESTORE_SEED = "tests::pipeline_restore_pg_tests::pipeline_restore_seed"
_RESTORE_RESUME = "tests::pipeline_restore_pg_tests::pipeline_restore_resume"
_RESTORE_CHECK = "pipeline_restore_drill"


class _RestoreDrillCase(_CorpusRunCase):
    """Fakes for `restore-drill`: the seed writes a tree of artifact files
    and its fingerprint file; the resume writes one passing result carrying
    the safe blocker. Docker and psql go through `_environment_invoke`,
    which also records the dump file's mode when `pg_dump` runs."""

    def setUp(self):
        super().setUp()
        self.dump_modes = []

    def _invoke(self, command, *, env, capture=False, input_text=None, log_path=None):
        self.calls.append(("invoke", list(command), input_text))
        if command[0] == "pg_dump":
            dump_path = Path(command[command.index("-f") + 1])
            self.dump_modes.append(dump_path.stat().st_mode & 0o777)
        return (0, "42") if capture else (0, None)

    def _fake_cargo(self, **overrides):
        def fake_cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            if test_filter == _RESTORE_SEED:
                root = Path(env["TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT"])
                (root / "objects" / "aa").mkdir(parents=True)
                (root / "objects" / "aa" / "first.bin").write_bytes(b"\x00first-ciphertext")
                (root / "second.bin").write_bytes(b"second-ciphertext")
                fingerprint = {
                    "schema": "trace_commons.pipeline_restore_fingerprint.v1",
                    "database_fingerprint": _fake_hash("database"),
                    "artifact_fingerprint": pipeline.artifact_fingerprint(root),
                    "index_entry_set_hash": _fake_hash("index"),
                    "pending_run_id_hash": _fake_hash("pending-run"),
                    "adapter_request_count": 1,
                    "completed_settlement_count": 1,
                    "completed_credit_event_count": 1,
                    "rls_table_count": 96,
                    "rls_policy_set_hash": _fake_hash("policies"),
                    "rls_policy_count": 150,
                    "rls_flag_set_hash": _fake_hash("flags"),
                    "rls_flag_table_count": 140,
                    "runtime_privilege_set_hash": _fake_hash("privileges"),
                    "runtime_privilege_count": 500,
                    "tenant_fingerprint": _fake_hash("tenants"),
                    "tenant_count": 2,
                    "audit_event_count": 2,
                }
                fingerprint.update(overrides.get("fingerprint", {}))
                Path(env["TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH"]).write_bytes(
                    results.canonical(fingerprint) + b"\n"
                )
            elif test_filter == _RESTORE_RESUME:
                seed = json.loads(Path(env["TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH"]).read_bytes())
                evidence = {
                    "database_fingerprint": seed["database_fingerprint"],
                    "artifact_fingerprint": seed["artifact_fingerprint"],
                    "index_entry_set_hash": seed["index_entry_set_hash"],
                    "pending_runs_resumed": 1,
                    "duplicate_effects": 0,
                    "rls_tables_checked": seed["rls_table_count"],
                    "rls_policy_set_hash": seed["rls_policy_set_hash"],
                    "rls_policy_count": seed["rls_policy_count"],
                    "rls_flag_set_hash": seed["rls_flag_set_hash"],
                    "rls_flag_table_count": seed["rls_flag_table_count"],
                    "runtime_privilege_set_hash": seed["runtime_privilege_set_hash"],
                    "tenant_fingerprint": seed["tenant_fingerprint"],
                    "tenant_count": seed["tenant_count"],
                    "audit_events_verified": seed["audit_event_count"],
                }
                evidence.update(overrides.get("evidence", {}))
                raw = {
                    "schema": results.SCHEMA,
                    "run_id": env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"],
                    "check_id": _RESTORE_CHECK,
                    "status": "pass",
                    "code_revision_hash": env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"],
                    "package_hash": _fake_hash("package"),
                    "configuration_digest": _fake_hash("configuration"),
                    "dependency_digest": _fake_hash("dependency"),
                    "observed_at": _iso(datetime.now(timezone.utc)),
                    "evidence_hash": _digest(results.canonical(evidence)),
                    "safe_blockers": overrides.get("safe_blockers", ["filesystem_restore_local_only"]),
                }
                result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
                (result_dir / f"{_RESTORE_CHECK}.result.json").write_text(json.dumps(raw))
                (result_dir / f"{_RESTORE_CHECK}.evidence.json").write_text(json.dumps(evidence))

        return fake_cargo_test

    def _drill(self, copy=None, **overrides):
        real_copy = pipeline.copy_artifacts

        def recording_copy(source, destination):
            self.calls.append(("copy", Path(source), Path(destination)))
            (copy or real_copy)(source, destination)

        with mock.patch.object(pipeline, "copy_artifacts", recording_copy), mock.patch.object(
            environment, "_invoke", self._invoke
        ):
            return self._main(
                ["restore-drill", "--postgres-admin-url", _ADMIN_URL], cargo=self._fake_cargo(**overrides)
            )

    def _main(self, argv, cargo=None, export=None):
        # Unlike `_CorpusRunCase._main`, leaves `_invoke` alone: `_drill`
        # patches it with this case's recording fake.
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(pipeline, "LOCAL_DIR", self.tmp / "local"))
            stack.enter_context(mock.patch.object(pipeline, "cargo_test", cargo))
            run_class = stack.enter_context(mock.patch.object(pipeline, "Run"))
            run_class.create.return_value = self.run
            stack.enter_context(mock.patch.object(environment, "_code_revision_hash", self._tree_hash))
            stack.enter_context(contextlib.redirect_stdout(self.stdout))
            stack.enter_context(contextlib.redirect_stderr(self.stderr))
            return pipeline.main(argv)

    def _events(self):
        """The drill's steps in the order they ran, as labels."""
        events = []
        for call in self.calls:
            if call[0] == "cargo":
                events.append(("cargo", call[3]))
            elif call[0] == "copy":
                events.append(("copy",))
            elif call[1][0] in ("pg_dump", "pg_restore"):
                events.append((call[1][0],))
            elif call[2] and "_restored;" in call[2] and "CREATE DATABASE" in call[2]:
                events.append(("createdb",))
        return events


class RestoreDrillTests(_RestoreDrillCase):
    def test_restore_drill_order(self):
        code = self._drill()
        self.assertEqual(code, 0, self.stderr.getvalue())
        self.assertEqual(
            self._events(),
            [
                ("cargo", _RESTORE_SEED),
                ("pg_dump",),
                ("createdb",),
                ("pg_restore",),
                ("copy",),
                ("cargo", _RESTORE_RESUME),
            ],
        )

        cargo_calls = self._cargo_calls()
        seed_env, resume_env = cargo_calls[0][4], cargo_calls[1][4]
        for call in cargo_calls:
            self.assertEqual(call[2], _INGEST_ARGS)
            self.assertTrue(call[5] and call[6], "both tests run --exact --ignored")
        # The seed gets the scenario's database (it creates `<db>_pilot`);
        # the resume gets the restored database itself.
        seed_database = seed_env["TRACE_COMMONS_PG_TEST_DATABASE_URL"].rsplit("/", 1)[1]
        resume_database = resume_env["TRACE_COMMONS_PG_TEST_DATABASE_URL"].rsplit("/", 1)[1]
        self.assertRegex(seed_database, r"^admission_test_[0-9a-f]{8}_01$")
        self.assertEqual(resume_database, f"{seed_database}_restored")
        dump = next(call[1] for call in self.calls if call[0] == "invoke" and call[1][0] == "pg_dump")
        restore = next(call[1] for call in self.calls if call[0] == "invoke" and call[1][0] == "pg_restore")
        self.assertEqual(dump[-1], f"{seed_database}_pilot")
        self.assertEqual(restore[restore.index("-d") + 1], resume_database)
        self.assertEqual(self.dump_modes, [0o600])

        # One random key for both processes, never printed.
        key = seed_env["TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX"]
        self.assertRegex(key, r"^[0-9a-f]{64}$")
        self.assertEqual(resume_env["TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX"], key)
        self.assertEqual(
            seed_env["TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH"],
            resume_env["TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH"],
        )
        source_root = Path(seed_env["TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT"])
        restored_root = Path(resume_env["TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT"])
        self.assertNotEqual(source_root, restored_root)
        self.assertEqual(pipeline.artifact_fingerprint(source_root), pipeline.artifact_fingerprint(restored_root))
        # Only the resume emits a check result.
        self.assertNotIn("TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR", seed_env)
        self.assertEqual(resume_env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"], self.run.run_id)

        output = self.stdout.getvalue() + self.stderr.getvalue()
        self.assertNotIn(key, output)
        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(len(lines), 3, self.stdout.getvalue())
        self.assertTrue(lines[0].startswith("PipelineRestoreOK: "))
        self.assertIn(f"database={_fake_hash('database')}", lines[0])
        self.assertIn("pending_runs_resumed=1", lines[0])
        self.assertIn("legs_per_run=1 credit_events_per_run=1", lines[0])
        self.assertIn("duplicate_effects=0", lines[0])
        self.assertEqual(
            lines[1],
            f"PipelineRestoreChecks: rls_tables=96 rls_policies={_fake_hash('policies')} rls_policy_count=150 "
            f"rls_flags={_fake_hash('flags')} rls_flag_tables=140 "
            f"runtime_privileges={_fake_hash('privileges')} "
            f"runtime_privilege_count=500 tenants=2 tenant_fingerprint={_fake_hash('tenants')} "
            "audit_events_verified=2",
        )
        self.assertIn("filesystem_restore_local_only", lines[2])
        self.assertIn("local evidence", lines[2])
        # The dump does not outlive the restore.
        self.assertEqual(list(self.run.run_dir.glob("*.dump")), [])

    def test_a_changed_artifact_byte_fails_before_the_resume(self):
        def flipping_copy(source, destination):
            shutil.copytree(source, destination)
            target = Path(destination) / "second.bin"
            data = bytearray(target.read_bytes())
            data[0] ^= 0x01
            target.write_bytes(bytes(data))

        code = self._drill(copy=flipping_copy)
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: restore_artifact_bytes_mismatch")
        self.assertEqual(
            [event for event in self._events() if event[0] == "cargo"],
            [("cargo", _RESTORE_SEED)],
            "the resume never runs",
        )
        self.assertNotIn("PipelineRestoreOK", self.stdout.getvalue())

        for label, mutate in (
            ("missing_file", lambda root: (root / "second.bin").unlink()),
            ("extra_file", lambda root: (root / "extra.bin").write_bytes(b"")),
        ):
            with self.subTest(label=label):
                shutil.rmtree(self.run.run_dir)
                self.run = _scratch_run("restore")
                self.calls.clear()
                self.stdout, self.stderr = io.StringIO(), io.StringIO()

                def mutating_copy(source, destination, mutate=mutate):
                    shutil.copytree(source, destination)
                    mutate(Path(destination))

                self.assertEqual(self._drill(copy=mutating_copy), 1)
                self.assertEqual(
                    self.stderr.getvalue().strip(), "PipelineFailure: restore_artifact_bytes_mismatch"
                )

    def test_restore_refuses_evidence_that_does_not_match_the_seed(self):
        cases = (
            ({"evidence": {"database_fingerprint": _fake_hash("other")}}, "restore_evidence_mismatch"),
            ({"evidence": {"duplicate_effects": 1}}, "restore_evidence_mismatch"),
            ({"evidence": {"pending_runs_resumed": 0}}, "restore_evidence_mismatch"),
            # Zaki's review of #1166, Major 2: the new checks' evidence must
            # be the seed's too.
            ({"evidence": {"rls_tables_checked": 95}}, "restore_evidence_mismatch"),
            # Review of this wave, I1: the whole policy set, not only the
            # named tenant policy.
            ({"evidence": {"rls_policy_set_hash": _fake_hash("other")}}, "restore_evidence_mismatch"),
            ({"evidence": {"rls_policy_count": 151}}, "restore_evidence_mismatch"),
            ({"fingerprint": {"rls_policy_count": 0}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"rls_policy_set_hash": "sha256:short"}}, "restore_fingerprint_invalid"),
            # Zaki's re-review of #1166, Medium: every table's RLS flags, not
            # only the pipeline tables and `TRACE_COMMONS_RLS_TABLES`.
            ({"evidence": {"rls_flag_set_hash": _fake_hash("other")}}, "restore_evidence_mismatch"),
            ({"evidence": {"rls_flag_table_count": 139}}, "restore_evidence_mismatch"),
            ({"fingerprint": {"rls_flag_table_count": 0}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"rls_flag_set_hash": "sha256:short"}}, "restore_fingerprint_invalid"),
            ({"evidence": {"runtime_privilege_set_hash": _fake_hash("other")}}, "restore_evidence_mismatch"),
            ({"evidence": {"tenant_fingerprint": _fake_hash("other")}}, "restore_evidence_mismatch"),
            ({"evidence": {"tenant_count": 1}}, "restore_evidence_mismatch"),
            ({"evidence": {"audit_events_verified": 1}}, "restore_evidence_mismatch"),
            ({"fingerprint": {"tenant_count": 1}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"audit_event_count": 0}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"tenant_fingerprint": "sha256:short"}}, "restore_fingerprint_invalid"),
            ({"safe_blockers": []}, "restore_safe_blocker_missing"),
            ({"fingerprint": {"tenant_id": "tenant-a"}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"completed_credit_event_count": 0}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"completed_settlement_count": True}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {"artifact_fingerprint": _fake_hash("not-the-tree")}},
             "restore_artifact_fingerprint_mismatch"),
        )
        for overrides, label in cases:
            with self.subTest(label=label, overrides=overrides):
                shutil.rmtree(self.run.run_dir)
                self.run = _scratch_run("restore")
                self.calls.clear()
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                self.assertEqual(self._drill(**overrides), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")

    def test_artifact_fingerprint_matches_the_rust_seed(self):
        # The same tree and value as `artifact_fingerprint_hashes_sorted_
        # relative_paths_and_bytes` in `pipeline_restore_pg_tests.rs`.
        root = self.tmp / "tree"
        (root / "a").mkdir(parents=True)
        (root / "z.bin").write_bytes(b"")
        (root / "a" / "b.bin").write_bytes(b"one")
        (root / "a-c.bin").write_bytes(b"two")
        self.assertEqual(
            pipeline.artifact_fingerprint(root),
            "sha256:9be20bc7d0122e748a727bf0433d393ba50edb9e2be45a3e106ab10e52be2716",
        )


class RestorePrivilegeTests(unittest.TestCase):
    def _commands(self, postgres_admin_url):
        calls = []

        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            calls.append(list(command))
            if command[:2] == ["docker", "port"]:
                return 0, "127.0.0.1:54321\n"
            return (0, "") if capture else (0, None)

        run = _scratch_run("restore")
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with environment.Environment(run, postgres_admin_url=postgres_admin_url) as env:
                    dump_path = run.run_dir / f"{run.run_id}.dump"
                    env.dump("admission_test_cafebabe_01_pilot", dump_path)
                    mode = dump_path.stat().st_mode & 0o777 if dump_path.exists() else None
                    env.create_database("admission_test_cafebabe_01_restored")
                    env.restore(dump_path, "admission_test_cafebabe_01_restored")
            container = f"tc-pipeline-{run.run_id}"
            return calls, dump_path, mode, container
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)

    def test_restore_uses_privileges_and_the_same_cluster(self):
        refused_flags = ("--no-privileges", "--no-acl", "-x", "--no-owner", "-O")

        calls, dump_path, _, container = self._commands(None)
        [dump] = [call for call in calls if "pg_dump" in call]
        [restore] = [call for call in calls if "pg_restore" in call]
        inner = f"/tmp/{dump_path.name}"
        self.assertEqual(
            dump,
            ["docker", "exec", container, "pg_dump", "-U", "trace", "-Fc", "-f", inner,
             "admission_test_cafebabe_01_pilot"],
        )
        self.assertEqual(
            restore,
            ["docker", "exec", container, "pg_restore", "-U", "trace", "-d",
             "admission_test_cafebabe_01_restored", inner],
        )
        for flag in refused_flags:
            self.assertNotIn(flag, restore)
        self.assertTrue(
            any(
                "psql" in call and container in call
                for call in calls
            ),
            "the restored database is created in the same container",
        )

        calls, dump_path, mode, _ = self._commands(_ADMIN_URL)
        [dump] = [call for call in calls if call[0] == "pg_dump"]
        [restore] = [call for call in calls if call[0] == "pg_restore"]
        cluster = ["-h", "127.0.0.1", "-p", "55431", "-U", "trace"]
        self.assertEqual(
            dump, ["pg_dump", *cluster, "-Fc", "-f", str(dump_path), "admission_test_cafebabe_01_pilot"]
        )
        self.assertEqual(
            restore, ["pg_restore", *cluster, "-d", "admission_test_cafebabe_01_restored", str(dump_path)]
        )
        for flag in refused_flags:
            self.assertNotIn(flag, restore)
        self.assertEqual(mode, 0o600, "the host dump file is private to its owner")

    def test_a_failed_restore_is_a_step_failure_with_a_log(self):
        def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
            if command[0] == "pg_restore":
                Path(log_path).write_text("pg_restore: error: could not execute query\n")
                return 1, None
            return (0, "") if capture else (0, None)

        run = _scratch_run("restore")
        try:
            with mock.patch.object(environment, "_invoke", fake_invoke):
                with environment.Environment(run, postgres_admin_url=_ADMIN_URL) as env:
                    with self.assertRaises(errors.StepFailed) as ctx:
                        env.restore(run.run_dir / "x.dump", "admission_test_cafebabe_01_restored")
            self.assertEqual(ctx.exception.step, "database_restore")
            self.assertEqual(ctx.exception.exit_code, 1)
            self.assertTrue(ctx.exception.log_path.is_file())
            with self.assertRaises(errors.ToolingError) as ctx:
                with mock.patch.object(environment, "_invoke", fake_invoke):
                    with environment.Environment(run, postgres_admin_url=_ADMIN_URL) as env:
                        env.create_database("Robert'); DROP TABLE")
            self.assertEqual(str(ctx.exception), "pipeline_tooling_database_name_invalid")
        finally:
            shutil.rmtree(run.run_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Task 11: the required checks, `pipeline.py qualify`, its report, and the
# catalog.
# ---------------------------------------------------------------------------

_PROMOTION_SOURCE = environment.ROOT / "crates/trace-commons-server/src/versioned_pipeline_qualification.rs"
_PROMOTION_ONLY = frozenset(
    {"pipeline_production_adapters", "pipeline_remote_restore", "pipeline_hf_network_canary"}
)
_QUALIFICATION_BLOCKERS = (
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
    "filesystem_restore_local_only",
    "hf_network_canary_not_run",
)
_CONTRACT_MANIFEST = (
    environment.ROOT / "docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json"
)
_INVENTORY_SCRIPT = "pipeline-deployment-inventory.py"


def _promotion_required_checks():
    """`PROMOTION_REQUIRED_CHECKS`, read out of the Rust source."""
    source = _PROMOTION_SOURCE.read_text()
    match = re.search(r"pub const PROMOTION_REQUIRED_CHECKS: &\[&str\] = &\[(.*?)\];", source, re.S)
    if match is None:
        raise AssertionError("PROMOTION_REQUIRED_CHECKS not found")
    return re.findall(r'"([a-z0-9_]+)"', match.group(1))


def _emit_check(env, check_id, *, digests=True, evidence=None, **overrides):
    """One passing result and its evidence, as `PipelineCheckEmitter` writes
    them from the variables `pipeline.py` sets."""
    evidence = {"observed": 1} if evidence is None else evidence
    raw = {
        "schema": results.SCHEMA,
        "run_id": env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"],
        "check_id": check_id,
        "status": "pass",
        "code_revision_hash": env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"],
        "package_hash": _fake_hash("package") if digests else None,
        "configuration_digest": _fake_hash("configuration") if digests else None,
        "dependency_digest": _fake_hash("dependency") if digests else None,
        "observed_at": _iso(datetime.now(timezone.utc)),
        "evidence_hash": _digest(results.canonical(evidence)),
        "safe_blockers": [],
    }
    raw.update(overrides)
    result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
    (result_dir / f"{check_id}.result.json").write_text(json.dumps(raw))
    (result_dir / f"{check_id}.evidence.json").write_text(json.dumps(evidence))


def _hf_manifest(bootstrap_bytes, holdout_bytes):
    """The export binary's `source-manifest.json` for these two corpora."""
    return {
        "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
        "source": {
            "repository": "jedisct1/security-audits",
            "revision": "6d527ff0081eec6704c2a4f00e1ef8d308ae7366",
            "split": "train",
            "translator": "swival",
            "bootstrap_count": 1,
            "holdout_count": 1,
            "min_words": 1,
            "max_words": 2000,
            "expected_instrument_count": 1,
        },
        "source_digest": _fake_hash("source"),
        "configuration_digest": _fake_hash("hf-configuration"),
        "order_digest": _fake_hash("order"),
        "bootstrap_corpus_digest": _digest(bootstrap_bytes),
        "holdout_corpus_digest": _digest(holdout_bytes),
        "sample_count": 2,
        "bootstrap_count": 1,
        "holdout_count": 1,
        "contains_raw_trace_text": False,
        "contains_contributor_identity": False,
    }


class RequiredCheckTests(unittest.TestCase):
    def test_required_checks_are_the_promotion_checks_less_the_promotion_only_ones(self):
        promotion = _promotion_required_checks()
        self.assertEqual(len(promotion), len(set(promotion)), "no duplicate promotion id")
        # Equality, not a subset (Zaki's review of #1166, nit 1): a Rust
        # promotion check that `qualify` has no step for fails here, as does
        # a `qualify` check the Rust list does not name.
        self.assertTrue(_PROMOTION_ONLY.issubset(promotion))
        self.assertEqual(
            checks.REQUIRED_CHECK_IDS,
            frozenset(promotion) - _PROMOTION_ONLY,
            (
                sorted(frozenset(promotion) - _PROMOTION_ONLY - checks.REQUIRED_CHECK_IDS),
                sorted(checks.REQUIRED_CHECK_IDS - frozenset(promotion)),
            ),
        )
        self.assertEqual(checks.REQUIRED_CHECK_IDS & _PROMOTION_ONLY, frozenset())
        self.assertNotIn("pipeline_http_corpus_package", checks.REQUIRED_CHECK_IDS)

        database_ids = [check.check_id for check in checks.REQUIRED_DATABASE_CHECKS]
        self.assertEqual(len(database_ids), 12)
        self.assertEqual(len(set(database_ids)), 12)
        self.assertEqual(
            checks.REQUIRED_CHECK_IDS,
            frozenset(database_ids) | set(checks.REQUIRED_CORPUS_CHECK_IDS) | {checks.RESTORE_CHECK_ID},
        )
        for check in checks.REQUIRED_DATABASE_CHECKS:
            with self.subTest(check=check.check_id):
                self.assertRegex(check.check_id, r"^[a-z0-9_]{1,64}$")
                self.assertIn(check.database, ("upgrade", "runtime", "pilot"))
                self.assertEqual(check.digests, check.database != "upgrade")


class _QualifyCase(_RestoreDrillCase):
    """Fakes for `qualify`. Every binding step succeeds, and the inventory
    script writes its output. Each database check's exact test emits its
    own check id from the variables it receives (unless named in
    `self.silent`). The corpus harness and the restore drill leave what
    their Task 9 and Task 10 fakes leave, and the HF export writes two
    corpora and their manifest."""

    def setUp(self):
        super().setUp()
        self.silent = set()
        self.foreign = set()
        self.empty = set()
        self.interrupt = None
        self.harness_emits = True
        self.fail_lock_drop = False

    def _invoke(self, command, *, env, capture=False, input_text=None, log_path=None):
        if any(str(part).endswith(_INVENTORY_SCRIPT) for part in command) and "--output" in command:
            Path(command[command.index("--output") + 1]).write_text(
                json.dumps(
                    {
                        "schema": "trace_commons.pipeline_deployment_inventory.v1",
                        "inventory_digest": _fake_hash("inventory"),
                    }
                )
            )
        if self.fail_lock_drop and input_text and "DROP DATABASE IF EXISTS pipeline_tooling_lock" in input_text:
            self.calls.append(("invoke", list(command), input_text))
            return (1, "") if capture else (1, None)
        return super()._invoke(command, env=env, capture=capture, input_text=input_text, log_path=log_path)

    def _fake_cargo(self, **overrides):
        restore = super()._fake_cargo(**overrides)
        by_test = {check.test_name: check for check in checks.REQUIRED_DATABASE_CHECKS}

        def fake_cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            if test_filter in (_RESTORE_SEED, _RESTORE_RESUME):
                restore(run, step, cargo_args, test_filter, env, exact=exact, ignored=ignored)
                return
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            if test_filter == _HARNESS:
                _write_harness_outputs(env, emit=self.harness_emits)
            elif test_filter in by_test and "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR" in env:
                check = by_test[test_filter]
                if check.check_id == self.interrupt:
                    raise KeyboardInterrupt
                if check.check_id in self.empty:
                    # What the emitter's `create_new` reservation leaves when
                    # the test dies before its final rename.
                    result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
                    (result_dir / f"{check.check_id}.result.json").write_text("")
                elif check.check_id in self.foreign:
                    _emit_check(env, check.check_id, digests=check.digests, run_id="qforeign0")
                elif check.check_id not in self.silent:
                    _emit_check(env, check.check_id, digests=check.digests)

        return fake_cargo_test

    def _fake_export(self, run, path, env, *, local_dir=None):
        self.calls.append(("export", Path(path), local_dir))
        output = run.run_dir / "hf"
        output.mkdir(parents=True, exist_ok=True)
        bootstrap = output / "bootstrap-corpus.json"
        holdout = output / "holdout-corpus.json"
        bootstrap.write_text(json.dumps(_direct_corpus(["hf_bootstrap_0000"], prefix="b")))
        holdout.write_text(json.dumps(_direct_corpus(["hf_holdout_0000"], prefix="h")))
        manifest = _hf_manifest(bootstrap.read_bytes(), holdout.read_bytes())
        (output / "source-manifest.json").write_text(json.dumps(manifest, indent=2))
        return [bootstrap, holdout]

    def _qualify(self, *extra):
        with mock.patch.object(environment, "_invoke", self._invoke), mock.patch.object(
            pipeline, "export_hf_corpus", self._fake_export
        ):
            return self._main(["qualify", "--postgres-admin-url", _ADMIN_URL, *extra], cargo=self._fake_cargo())

    def _fresh_run(self):
        shutil.rmtree(self.run.run_dir, ignore_errors=True)
        self.run = _scratch_run("qualify")
        self.calls.clear()
        self.stdout, self.stderr = io.StringIO(), io.StringIO()

    @property
    def report_path(self):
        return self.tmp / "local" / "pipeline-qualification-report.json"

    @property
    def catalog_path(self):
        return self.tmp / "local" / "pipeline-lab-catalog.json"


class QualifyTests(_QualifyCase):
    def test_qualify_fails_when_a_required_check_emits_nothing(self):
        self.silent = {"pipeline_payout_recovery"}
        code = self._qualify()
        self.assertEqual(code, 1)
        self.assertEqual(
            self.stderr.getvalue().strip(), "PipelineFailure: check_result_missing:pipeline_payout_recovery"
        )
        self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())
        value = json.loads(self.report_path.read_text())
        self.assertEqual(value["status"], "fail")
        self.assertEqual(value["failure"], "check_result_missing:pipeline_payout_recovery")
        self.assertIs(value["production_promotion_ready"], False)
        self.assertNotIn("pipeline_payout_recovery", {item["check_id"] for item in value["checks"]})
        results.validate_evidence(value)
        self.assertEqual(
            (self.run.run_dir / "qualification-report.json").read_bytes(), self.report_path.read_bytes()
        )
        self.assertNotIn(_HARNESS, [call[3] for call in self._cargo_calls()], "qualify stops at the first failure")
        self.assertFalse(self.catalog_path.exists())

        with self.subTest("a result from another run is refused and left out of the report"):
            self._fresh_run()
            self.silent = set()
            self.foreign = {"pipeline_receipt_replay_exact"}
            self.assertEqual(self._qualify(), 1)
            self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: check_result_foreign_run")
            value = json.loads(self.report_path.read_text())
            self.assertEqual((value["status"], value["failure"]), ("fail", "check_result_foreign_run"))
            reported = {item["check_id"] for item in value["checks"]}
            self.assertNotIn("pipeline_receipt_replay_exact", reported)
            self.assertIn("pipeline_stale_lease_fence", reported, "this run's earlier results stay")
            self.foreign = set()

        with self.subTest("a corpus run that emits nothing"):
            self._fresh_run()
            self.silent = set()
            self.harness_emits = False
            self.assertEqual(self._qualify("--archive"), 1)
            self.assertEqual(
                self.stderr.getvalue().strip(), "PipelineFailure: check_result_missing:pipeline_http_corpus_minimal"
            )
            value = json.loads(self.report_path.read_text())
            self.assertEqual(value["status"], "fail")
            self.assertEqual(value["failure"], "check_result_missing:pipeline_http_corpus_minimal")
            self.assertFalse(self.catalog_path.exists(), "a failed qualify archives nothing")

    def test_qualify_runs_each_check_in_a_fresh_scenario(self):
        code = self._qualify()
        self.assertEqual(code, 0, self.stderr.getvalue())
        cargo_indexes = [index for index, call in enumerate(self.calls) if call[0] == "cargo"]
        by_test = {check.test_name: check for check in checks.REQUIRED_DATABASE_CHECKS}
        databases = []
        steps = []
        for position, index in enumerate(cargo_indexes):
            call = self.calls[index]
            steps.append(call[1])
            if call[3] not in by_test:
                continue
            check = by_test[call[3]]
            _, step, cargo_args, _, env, exact, ignored = call
            self.assertEqual(
                (step, cargo_args, exact, ignored), (check.check_id, check.cargo_args, True, check.ignored)
            )
            self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"], self.run.run_id)
            self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"], str(self.run.results_dir))
            self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"], self.run.code_revision_hash)
            if check.database == "upgrade":
                database = env["TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"].rsplit("/", 1)[1]
                self.assertRegex(database, r"^pipeline_test_[0-9a-f]{8}_[0-9]{2}$")
                self.assertNotIn("TRACE_COMMONS_PG_TEST_DATABASE_URL", env)
                guarded = database
            else:
                database = env["TRACE_COMMONS_PG_TEST_DATABASE_URL"].rsplit("/", 1)[1]
                self.assertRegex(database, r"^admission_test_[0-9a-f]{8}_[0-9]{2}$")
                self.assertNotIn("TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL", env)
                guarded = f"{database}_pilot" if check.database == "pilot" else database
            self.assertNotIn(database, databases, f"{check.check_id} reuses a scenario")
            databases.append(database)
            created = [
                item for item in self.calls[:index]
                if item[0] == "invoke" and item[2] and f"CREATE DATABASE {database};" in item[2]
            ]
            self.assertEqual(len(created), 1, f"{check.check_id} gets its own new database")
            following = cargo_indexes[position + 1] if position + 1 < len(cargo_indexes) else len(self.calls)
            guards = [
                item for item in self.calls[index + 1:following]
                if item[0] == "invoke" and item[2] and "pg_stat_database" in item[2] and f"'{guarded}'" in item[2]
            ]
            self.assertEqual(len(guards), 1, f"the xact guard runs after {check.check_id}")
        self.assertEqual(len(databases), 12)

        # The binding checks run first, then migration atomicity in its own
        # scenario, then the database checks in table order, the three
        # corpus runs, and the restore drill.
        binding = [step.step for step in (*checks.CONTRACTS_STEPS, *checks.RUNTIME_STEPS) if step.kind == "cargo"]
        database_steps = [check.check_id for check in checks.REQUIRED_DATABASE_CHECKS]
        self.assertEqual(
            steps,
            [
                *binding,
                "postgres_migration_atomicity",
                *database_steps,
                *checks.REQUIRED_CORPUS_CHECK_IDS,
                "restore_seed",
                "restore_resume",
            ],
        )
        inventory = [
            item[1] for item in self.calls
            if item[0] == "invoke" and any(str(part).endswith(_INVENTORY_SCRIPT) for part in item[1])
        ]
        self.assertEqual(len(inventory), 1)
        self.assertEqual(inventory[0][-3:], ["--check", "--output", str(self.run.run_dir / "inventory.json")])
        self.assertTrue(
            any(
                item[0] == "invoke" and any(str(part).endswith("test_pipeline_tooling.py") for part in item[1])
                for item in self.calls
            ),
            "the tooling self-tests are a binding check",
        )
        # One environment for the whole command.
        locks = [
            item for item in self.calls
            if item[0] == "invoke" and item[2] and "CREATE DATABASE pipeline_tooling_lock" in item[2]
        ]
        self.assertEqual(len(locks), 1)
        # The corpus runs each get their own scenario too, and no corpus run
        # reuses a database check's.
        corpus_databases = [
            call[4]["TRACE_COMMONS_PG_TEST_DATABASE_URL"].rsplit("/", 1)[1]
            for call in self._cargo_calls() if call[3] == _HARNESS
        ]
        self.assertEqual(len(set(corpus_databases)), 3)
        self.assertEqual(set(corpus_databases) & set(databases), set())
        self.assertEqual(
            [call[4]["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"] for call in self._cargo_calls() if call[3] == _HARNESS],
            list(checks.REQUIRED_CORPUS_CHECK_IDS),
        )

    def test_routine_runs_do_not_archive(self):
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        self.assertTrue(self.report_path.is_file())
        self.assertFalse(self.catalog_path.exists(), "a routine qualify writes no catalog")
        self.assertFalse((self.catalog_path.parent / "lab-records").exists())

        self._fresh_run()
        self.assertEqual(self._qualify("--archive"), 0, self.stderr.getvalue())
        value = json.loads(self.catalog_path.read_text())
        self.assertEqual(value["schema"], "trace_commons.pipeline_lab_catalog.v1")
        [entry] = value["qualifications"]
        self.assertEqual(entry["status"], "pass")
        self.assertIs(entry["production_promotion_ready"], False)
        report_value = json.loads(self.report_path.read_text())
        self.assertEqual(entry["evidence_hash"], report_value["evidence_hash"])
        archived = self.catalog_path.parent / entry["report"]
        self.assertEqual(json.loads(archived.read_bytes()), report_value)
        self.assertEqual(archived.name, f"qualification-{_digest(archived.read_bytes())[7:]}.json")
        self.assertEqual(len(entry["records"]), 4, "three corpus reports and the HF manifest")
        schemas = sorted(
            json.loads((self.catalog_path.parent / path).read_bytes())["schema"] for path in entry["records"]
        )
        self.assertEqual(
            schemas,
            ["trace_commons.pipeline_corpus_report.v1"] * 3 + ["trace_commons.pipeline_hf_corpus_manifest.v1"],
        )
        for path in entry["records"]:
            data = (self.catalog_path.parent / path).read_bytes()
            self.assertEqual(Path(path).name, f"evidence-{_digest(data)[7:]}.json")

        # Archiving the same report and records again changes nothing.
        before = self.catalog_path.read_bytes()
        records = pipeline.corpus_records(self.run)
        self.assertEqual(len(records), 4)
        catalog.update_catalog(self.catalog_path, self.report_path, records=records)
        self.assertEqual(self.catalog_path.read_bytes(), before)

        # A record whose bytes changed at an existing digest is refused, and
        # the catalog is left as it was.
        changed = self.catalog_path.parent / entry["records"][0]
        changed.write_bytes(changed.read_bytes() + b" ")
        with self.assertRaises(errors.ToolingError) as ctx:
            catalog.update_catalog(self.catalog_path, self.report_path, records=records)
        self.assertEqual(str(ctx.exception), "immutable_record_conflict")
        self.assertEqual(self.catalog_path.read_bytes(), before)

        # A record that does not belong to the report is refused before any
        # file changes.
        other = self.tmp / "other-corpus-report.json"
        other.write_bytes(results.canonical(_corpus_report("pipeline_http_corpus_minimal", [
            ("corpus", _fake_hash("other-corpus"), [_fixture_report("other_fixture")])
        ])) + b"\n")
        with self.assertRaises(errors.ToolingError) as ctx:
            catalog.update_catalog(self.catalog_path, self.report_path, records=[other])
        self.assertEqual(str(ctx.exception), "qualification_corpus_mismatch")

    def test_report_contains_no_private_fields(self):
        code = self._qualify()
        self.assertEqual(code, 0, self.stderr.getvalue())
        data = self.report_path.read_bytes()
        value = json.loads(data)
        results.validate_evidence(value)
        report_module.validate_qualification_report(value)
        self.assertEqual(value["schema"], "trace_commons.pipeline_qualification_report.v1")
        self.assertEqual(value["scope"], "local_test")
        self.assertEqual(value["status"], "pass")
        self.assertIsNone(value["failure"])
        self.assertIs(value["production_promotion_ready"], False)
        self.assertIs(value["external_payout_enabled"], False)
        self.assertEqual(value["safe_blockers"], list(_QUALIFICATION_BLOCKERS))
        self.assertEqual({item["check_id"] for item in value["checks"]}, checks.REQUIRED_CHECK_IDS)
        [restore] = [item for item in value["checks"] if item["check_id"] == checks.RESTORE_CHECK_ID]
        self.assertEqual(restore["safe_blockers"], ["filesystem_restore_local_only"])
        for item in value["checks"]:
            self.assertEqual(item["status"], "pass")
            self.assertEqual(item["evidence_hash"], _digest(results.canonical(item["evidence"])))
        self.assertEqual(
            value["evidence_hash"],
            _digest(results.canonical({"inputs": value["inputs"], "checks": value["checks"]})),
        )
        inputs = value["inputs"]
        self.assertEqual(inputs["code_revision_hash"], self.run.code_revision_hash)
        self.assertEqual(inputs["inventory_digest"], _fake_hash("inventory"))
        self.assertEqual(inputs["contract_manifest_digest"], _digest(_CONTRACT_MANIFEST.read_bytes()))
        self.assertEqual(
            [run["check_id"] for run in inputs["corpus_runs"]], sorted(checks.REQUIRED_CORPUS_CHECK_IDS)
        )
        for run in inputs["corpus_runs"]:
            self.assertEqual(run["bundle_id"], _fake_hash("bundle"))
            self.assertEqual(run["package_hash"], _fake_hash("package"))
            self.assertRegex(run["report_digest"], r"^sha256:[0-9a-f]{64}$")
        [hf_run] = [run for run in inputs["corpus_runs"] if run["check_id"] == "pipeline_http_corpus_hf_local"]
        self.assertEqual(len(hf_run["corpus_digests"]), 2)

        text = data.decode()
        self.assertNotIn("postgres://", text)
        self.assertNotIn("tenant-a", text)
        self.assertNotIn("probe_", text)
        for call in self._cargo_calls():
            key = call[4].get("TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX")
            if key:
                self.assertNotIn(key, text)
        self.assertEqual((self.run.run_dir / "qualification-report.json").read_bytes(), data)
        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(lines[0], f"PipelineQualificationOK: report={self.report_path.name} checks=16")
        self.assertTrue(lines[1].startswith("PipelineQualificationScope: production_promotion_ready=false"))
        self.assertEqual(self.stderr.getvalue(), "")

        for label, tampered in (
            ("private field", {**value, "secret_probe": "x"}),
            ("promotion ready", {**value, "production_promotion_ready": True}),
            ("missing blocker", {**value, "safe_blockers": value["safe_blockers"][1:]}),
            ("changed input", {**value, "inputs": {**inputs, "inventory_digest": _fake_hash("other")}}),
            ("missing check", {**value, "checks": value["checks"][1:]}),
            (
                "a check blocker left out",
                {**value, "safe_blockers": [label for label in value["safe_blockers"]
                                            if label != "filesystem_restore_local_only"]},
            ),
        ):
            with self.subTest(label=label):
                with self.assertRaises(errors.ToolingError):
                    report_module.validate_qualification_report(tampered)

    def test_qualify_reports_an_empty_result_file_with_a_label(self):
        """Final review M1: an empty (or malformed) result file fails with
        `check_result_schema_invalid` and a fail report, not a traceback."""
        self.empty = {"pipeline_stale_lease_fence"}
        code = self._qualify()
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: check_result_schema_invalid")
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", "check_result_schema_invalid"))
        self.assertEqual(value["checks"], [], "results that no longer load are left out")

    def test_an_interrupted_qualify_writes_a_fail_report(self):
        """Final review M1 (deferred Task 11 M1): Ctrl-C during `qualify`
        writes the `status: fail` report under `qualify_interrupted`, with
        the results the run reached, then ends the command with 130."""
        self.interrupt = "pipeline_stale_lease_fence"
        code = self._qualify()
        self.assertEqual(code, 130)
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", "qualify_interrupted"))
        reported = {item["check_id"] for item in value["checks"]}
        self.assertIn("pipeline_independent_instruments", reported, "the results reached are kept")
        self.assertNotIn("pipeline_stale_lease_fence", reported)
        self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())

    def test_qualify_removes_the_previous_report_before_it_runs(self):
        """Final review M1: an older passing report never stays the latest
        report, even when a failed run cannot write its own."""
        self.report_path.parent.mkdir(parents=True, exist_ok=True)
        self.report_path.write_text('{"status": "pass"}')
        self.silent = {"pipeline_payout_recovery"}
        with mock.patch.object(pipeline, "write_report", side_effect=OSError("disk full")):
            code = self._qualify()
        self.assertEqual(code, 1)
        self.assertFalse(self.report_path.exists(), "the older report is gone")

    def test_qualify_reports_a_cleanup_failure(self):
        self.fail_lock_drop = True
        code = self._qualify("--archive")
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: cleanup_failed")
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", "cleanup_failed"))
        self.assertFalse(self.catalog_path.exists())


class CodeRevisionChangedTests(_QualifyCase):
    """Zaki's review of #1166, minor 1: the tree hash was taken once, at
    `Run.create`, so a file edited mid-run was credited to the tree the run
    started from. Each command that credits evidence to that hash computes
    it again at its end and fails `code_revision_changed` on a mismatch."""

    def test_qualify_fails_and_reports_an_edited_tree(self):
        self.tree_edited = True
        code = self._qualify("--archive")
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: code_revision_changed")
        self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", "code_revision_changed"))
        self.assertFalse(self.catalog_path.exists(), "nothing is archived")
        self.assertEqual(self.tree_hash_calls, 1, "computed again once, after every check ran")

    def test_restore_drill_fails_on_an_edited_tree(self):
        self.tree_edited = True
        code = self._drill()
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: code_revision_changed")
        self.assertNotIn("PipelineRestoreOK", self.stdout.getvalue())

    def test_run_fails_on_an_edited_tree_and_archives_nothing(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture", "beta_fixture"])))
        self.tree_edited = True
        code = _CorpusRunCase._main(
            self,
            ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL, "--archive"],
            cargo=_CorpusRunCase._fake_cargo(self),
        )
        self.assertEqual(code, 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: code_revision_changed")
        self.assertNotIn("PipelineRunOK", self.stdout.getvalue())
        self.assertFalse(self.catalog_path.exists())

    def test_an_unchanged_tree_passes_each_command(self):
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        self.assertEqual(self.tree_hash_calls, 1)
        self._fresh_run()
        self.assertEqual(self._drill(), 0, self.stderr.getvalue())
        self.assertEqual(self.tree_hash_calls, 2)


if __name__ == "__main__":
    unittest.main()
