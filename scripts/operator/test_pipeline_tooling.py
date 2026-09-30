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
import shutil
import tempfile
import unittest
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest import mock

import pipeline
from pipeline_tooling import cargo, checks, environment, errors, results


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


if __name__ == "__main__":
    unittest.main()
