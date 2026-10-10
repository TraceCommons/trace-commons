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
import importlib.util
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest import mock

import pipeline
from pipeline_tooling import cargo, catalog, checks, comparison, corpus, environment, errors, results
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


class OnePackageTests(unittest.TestCase):
    """P5-D15: a qualification run names exactly one package. The results
    that name a package agree on all three digests; a mechanics result
    names none."""

    def _result(self, check_id, *, package=None, **overrides):
        if package is not None:
            digests = {
                "package_hash": _fake_hash(f"{package}-package"),
                "configuration_digest": _fake_hash(f"{package}-configuration"),
                "dependency_digest": _fake_hash(f"{package}-dependency"),
            }
        else:
            digests = {"package_hash": None, "configuration_digest": None, "dependency_digest": None}
        fields = {
            "schema": results.SCHEMA,
            "run_id": "qcafebabe",
            "check_id": check_id,
            "status": "pass",
            "code_revision_hash": _fake_hash("code-revision"),
            "observed_at": datetime.now(timezone.utc),
            "evidence_hash": _fake_hash(check_id),
            "safe_blockers": (),
            **digests,
        }
        fields.update(overrides)
        return results.CheckResult(**fields)

    def _keyed(self, *items):
        return {item.check_id: item for item in items}

    def test_a_result_set_with_two_packages_is_reported_mixed(self):
        with self.subTest("two package hashes"):
            mixed = self._keyed(
                self._result("pipeline_bundle_qualification", package="candidate"),
                self._result("pipeline_http_corpus_compatibility", package="other"),
            )
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_one_package(mixed)
            self.assertEqual(str(ctx.exception), "qualification_evidence_mixed_package")

        with self.subTest("one digest that differs is a different package"):
            same_hash = self._result("pipeline_http_corpus_hf_local", package="candidate")
            other_configuration = self._result(
                "pipeline_restore_drill",
                package="candidate",
                configuration_digest=_fake_hash("other-configuration"),
            )
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_one_package(self._keyed(same_hash, other_configuration))
            self.assertEqual(str(ctx.exception), "qualification_evidence_mixed_package")

        with self.subTest("a lone configuration digest names a package, as in evaluate_promotion"):
            partial = self._result("pipeline_crash_matrix", configuration_digest=_fake_hash("candidate-configuration"))
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_one_package(
                    self._keyed(partial, self._result("pipeline_restore_drill", package="candidate"))
                )
            self.assertEqual(str(ctx.exception), "qualification_evidence_mixed_package")

        with self.subTest("package-bearing results agree and the others carry none"):
            results.require_one_package(
                self._keyed(
                    *(self._result(check_id, package="candidate") for check_id in _CANDIDATE_CHECKS),
                    self._result("pipeline_crash_matrix"),
                    self._result("pipeline_http_corpus_minimal"),
                )
            )  # must not raise

        with self.subTest("no result names a package"):
            results.require_one_package(self._keyed(self._result("pipeline_crash_matrix")))
            results.require_one_package({})

    def test_a_mechanics_result_may_not_name_a_package(self):
        """The tooling refuses a mechanics check that carries a digest (any
        one of the three), so a later check cannot name a test bundle again
        without a test failing."""
        for key in ("package_hash", "configuration_digest", "dependency_digest"):
            with self.subTest(key), tempfile.TemporaryDirectory() as tmp:
                run = _make_run(Path(tmp))
                digests = {"package_hash": None, "configuration_digest": None, "dependency_digest": None}
                digests[key] = _fake_hash("a-test-bundle")
                _write_check_files(
                    run.results_dir, "pipeline_crash_matrix", {"a": 1},
                    run_id=run.run_id, code_revision_hash=run.code_revision_hash, **digests,
                )
                loaded = results.load_results(run)
                required = {"pipeline_crash_matrix": checks.CheckSpec("pipeline_crash_matrix", digests_required=False)}
                with self.assertRaises(errors.ToolingError) as ctx:
                    results.require_current_pass_results(run, loaded, required)
                self.assertEqual(str(ctx.exception), "pipeline_check_digests_unexpected:pipeline_crash_matrix")

        with self.subTest("none carried passes"), tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            _write_check_files(
                run.results_dir, "pipeline_crash_matrix", {"a": 1},
                run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                package_hash=None, configuration_digest=None, dependency_digest=None,
            )
            required = {"pipeline_crash_matrix": checks.CheckSpec("pipeline_crash_matrix", digests_required=False)}
            results.require_current_pass_results(run, results.load_results(run), required)  # must not raise

    def test_a_candidate_check_still_needs_all_three_digests(self):
        """The other direction, for each of the four checks that test the
        candidate: a result without a digest is refused, whichever digest it
        lacks."""
        specs = checks.required_specs()
        for check_id in _CANDIDATE_CHECKS:
            for missing in ("package_hash", "configuration_digest", "dependency_digest"):
                with self.subTest(check_id=check_id, missing=missing), tempfile.TemporaryDirectory() as tmp:
                    run = _make_run(Path(tmp))
                    _write_check_files(
                        run.results_dir, check_id, {"a": 1},
                        run_id=run.run_id, code_revision_hash=run.code_revision_hash, **{missing: None},
                    )
                    with self.assertRaises(errors.ToolingError) as ctx:
                        results.require_current_pass_results(
                            run, results.load_results(run), {check_id: specs[check_id]}
                        )
                    self.assertEqual(str(ctx.exception), f"check_result_digest_missing:{check_id}")

    def test_require_current_pass_results_refuses_a_mixed_run(self):
        """`require_current_pass_results` applies the one-package rule to
        every result it is given, not only the ones it requires."""
        with tempfile.TemporaryDirectory() as tmp:
            run = _make_run(Path(tmp))
            for check_id, package in (
                ("pipeline_bundle_qualification", "candidate"),
                ("pipeline_http_corpus_compatibility", "other"),
            ):
                _write_check_files(
                    run.results_dir, check_id, {"a": 1},
                    run_id=run.run_id, code_revision_hash=run.code_revision_hash,
                    package_hash=_fake_hash(f"{package}-package"),
                )
            required = {"pipeline_bundle_qualification": checks.required_specs()["pipeline_bundle_qualification"]}
            with self.assertRaises(errors.ToolingError) as ctx:
                results.require_current_pass_results(run, results.load_results(run), required)
            self.assertEqual(str(ctx.exception), "qualification_evidence_mixed_package")


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
_ATTESTATION_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_check_attestations_write"
_KEY_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_signing_key_write"
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


_NO_DIGESTS = {"package_hash": None, "configuration_digest": None, "dependency_digest": None}


def _candidate_digests():
    """The three digests of the fake candidate package. Every fake that
    stands for a check that tests the candidate (the corpus harness, the
    restore resume, the runtime and HTTP database tests) names these, so a
    qualification run of the fakes has one package. The corpus report's
    configuration digest is derived from its manifest, so this one is too."""
    manifest = _policy_manifest()
    return {
        "package_hash": _fake_hash("package"),
        "configuration_digest": _digest(
            results.canonical({phase: manifest[phase]["configuration_hash"] for phase in corpus.PHASES})
        ),
        "dependency_digest": _fake_hash("dependency"),
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
        **_candidate_digests(),
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


_DROP = object()


def _harness_evidence_digests(partitions):
    """`(corpus_digest, input_digest)` as the corpus harness's evidence
    carries them: the one partition's corpus digest (or, for two, the
    canonical hash of the list of both), and the canonical hash of the list
    of every fixture's request-content hash, in order."""
    corpus_digests = [digest for _, digest, _ in partitions]
    corpus_digest = corpus_digests[0] if len(corpus_digests) == 1 else _digest(results.canonical(corpus_digests))
    requests = [item["request_content_hash"] for _, _, fixtures in partitions for item in fixtures]
    return corpus_digest, _digest(results.canonical(requests))


def _write_harness_outputs(
    env,
    fixtures_by_partition=None,
    emit=True,
    digests=None,
    result_overrides=None,
    evidence_overrides=None,
    report_assembly=None,
):
    """What `pipeline_corpus_run` leaves behind: the report at
    `TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH` and, when every fixture
    passed, one check result and its evidence in the result directory. The
    result names the corpus report's package (`digests=True`) or none
    (`digests=False`); by default only `pipeline_http_corpus_minimal`, which
    serves a test bundle, names none (P5-D15). `result_overrides` replaces
    fields of the result (a digest that differs from the report's). Under
    `TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY=production` the report and the
    evidence say `harness_assembly: production` and the report carries the
    production harness's blockers, as the Rust harness writes them;
    `report_assembly` forces the report's mode either way."""
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
    production = env.get("TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY") == "production"
    if (report_assembly or ("production" if production else "reference")) == "production":
        report["harness_assembly"] = "production"
        report["safe_blockers"] = list(corpus.PRODUCTION_HARNESS_BLOCKERS)
        report = _resigned(report)
    report_bytes = results.canonical(report) + b"\n"
    Path(env["TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH"]).write_bytes(report_bytes)
    if not emit:
        return report
    corpus_digest, input_digest = _harness_evidence_digests(partitions)
    evidence = {
        "fixtures": report["fixture_count"],
        "completed": report["completed_fixture_count"],
        "replay_same_run": report["replay_same_run_count"],
        "changed_content_refused": report["changed_content_refused_count"],
        "tenant_isolation": report["tenant_isolation"],
        "report_hash": _digest(report_bytes),
        "corpus_digest": corpus_digest,
        "input_digest": input_digest,
    }
    if production:
        evidence["harness_assembly"] = "production"
    for key, value in (evidence_overrides or {}).items():
        if value is _DROP:
            del evidence[key]
        else:
            evidence[key] = value
    if digests is None:
        digests = check_id != "pipeline_http_corpus_minimal"
    result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
    raw = {
        "schema": results.SCHEMA,
        "run_id": env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"],
        "check_id": check_id,
        "status": "pass",
        "code_revision_hash": env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"],
        "package_hash": report["package_hash"] if digests else None,
        "configuration_digest": report["configuration_digest"] if digests else None,
        "dependency_digest": report["dependency_digest"] if digests else None,
        "observed_at": _iso(datetime.now(timezone.utc)),
        "evidence_hash": _digest(results.canonical(evidence)),
        "safe_blockers": [],
    }
    raw.update(result_overrides or {})
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

    def test_run_names_a_package_only_for_a_candidate_check(self):
        """The minimal bundle is a test bundle, so its check result names no
        package (P5-D15); the compatibility check names the report's. The
        corpus report keeps its own digests either way."""
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        for bundle, named in (("minimal", False), ("compatibility", True)):
            with self.subTest(bundle=bundle):
                shutil.rmtree(self.run.run_dir)
                self.run = _scratch_run("corpus")
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                argv = ["run", "--bundle", bundle, "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL]
                self.assertEqual(self._main(argv, cargo=self._fake_cargo()), 0, self.stderr.getvalue())
                result = results.load_results(self.run)[f"pipeline_http_corpus_{bundle}"]
                report = json.loads((self.tmp / "local" / f"pipeline-{bundle}-corpus-report.json").read_text())
                self.assertEqual(report["package_hash"], _fake_hash("package"))
                self.assertIsNotNone(report["configuration_digest"])
                self.assertEqual(
                    (result.package_hash, result.configuration_digest, result.dependency_digest),
                    (report["package_hash"], report["configuration_digest"], report["dependency_digest"])
                    if named
                    else (None, None, None),
                )

    def test_run_refuses_a_result_whose_package_differs_from_its_report(self):
        """A check that names the package must name the one its corpus
        report says the run served: a result with another digest (one,
        whichever) fails with `corpus_report_package_mismatch`."""
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        for key in ("package_hash", "configuration_digest", "dependency_digest"):
            with self.subTest(key):
                shutil.rmtree(self.run.run_dir)
                self.run = _scratch_run("corpus")
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                argv = [
                    "run", "--bundle", "compatibility", "--corpus", str(corpus_path),
                    "--postgres-admin-url", _ADMIN_URL,
                ]
                cargo = self._fake_cargo(result_overrides={key: _fake_hash(f"another-{key}")})
                self.assertEqual(self._main(argv, cargo=cargo), 1)
                self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: corpus_report_package_mismatch")

    def test_run_refuses_digests_on_the_minimal_check_and_their_absence_elsewhere(self):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        for bundle, digests, label in (
            ("minimal", True, "pipeline_check_digests_unexpected:pipeline_http_corpus_minimal"),
            ("compatibility", False, "check_result_digest_missing:pipeline_http_corpus_compatibility"),
        ):
            with self.subTest(bundle=bundle):
                shutil.rmtree(self.run.run_dir)
                self.run = _scratch_run("corpus")
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                argv = ["run", "--bundle", bundle, "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL]
                self.assertEqual(self._main(argv, cargo=self._fake_cargo(digests=digests)), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")

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


class ProductionCorpusReportTests(unittest.TestCase):
    """Spec section 2, O-B3 (plan B3): a production-mode corpus report
    compares only the deterministic fields, says it is a production-mode
    report, and carries the harness's own blockers."""

    def _production(self, **fixture_overrides):
        report = _corpus_report(
            "pipeline_http_corpus_compatibility",
            [("corpus", _fake_hash("corpus"), [_fixture_report("alpha_fixture", **fixture_overrides)])],
        )
        report.pop("report_digest")
        report["harness_assembly"] = "production"
        report["safe_blockers"] = list(corpus.PRODUCTION_HARNESS_BLOCKERS)
        return _resigned(report)

    def test_production_corpus_mode_compares_only_deterministic_fields(self):
        moved = {
            "phase_count": 3,
            "scoring_state": "complete",
            "settlement_state": "incomplete",
            "instrument_count": 0,
            "instruments": [],
            "instrument_states": {},
        }
        item = _fixture_report("alpha_fixture", **moved)
        self.assertEqual(
            corpus.fixture_mismatches(item),
            ["outcome_count", "settlement_state", "instrument_count", "instrument_states"],
        )
        self.assertEqual(corpus.fixture_mismatches(item, "production"), [])
        corpus.validate_report(self._production(**moved))  # must not raise
        for field, value in (
            ("state", "awaiting_review"),
            ("admission_decision", "quarantine"),
            ("consent_state", "refused"),
            ("privacy_state", "medium"),
            ("replay_same_run", False),
            ("changed_content_refused", False),
            ("tenant_isolation", False),
        ):
            with self.subTest(field=field):
                changed = _fixture_report("alpha_fixture", **{field: value})
                self.assertEqual(len(corpus.fixture_mismatches(changed, "production")), 1)

        # A production report that names a scoring mismatch it no longer
        # compares does not match its own rule.
        with self.assertRaises(errors.ToolingError) as ctx:
            corpus.validate_report(self._production(**moved, mismatches=["settlement_state"]))
        self.assertEqual(str(ctx.exception), "qualification_mismatch_not_failed")

    def test_production_blockers_agree_with_the_harness(self):
        source = (
            environment.ROOT / "crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs"
        ).read_text()
        match = re.search(r"const PRODUCTION_HARNESS_BLOCKERS: \[&str; \d+\] = \[(.*?)\];", source, re.S)
        self.assertIsNotNone(match)
        self.assertEqual(tuple(re.findall(r'"([a-z0-9_]+)"', match.group(1))), corpus.PRODUCTION_HARNESS_BLOCKERS)

    def test_production_report_carries_its_own_blockers_and_mode(self):
        report = self._production()
        corpus.validate_report(report)  # must not raise
        cases = {
            "missing_local_blockers": _resigned(dict(report, safe_blockers=["local_test_only"])),
            "invalid_report_harness_assembly": _resigned(dict(report, harness_assembly="reference")),
            "invalid_report_harness_assembly:other": _resigned(dict(report, harness_assembly="staging")),
        }
        for case, value in cases.items():
            with self.subTest(case=case):
                with self.assertRaises(errors.ToolingError) as ctx:
                    corpus.validate_report(value)
                self.assertEqual(str(ctx.exception), case.split(":")[0])


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

    def test_package_production_passes_only_the_env_files_package_variables(self):
        """Spec B-D5: `package --bundle production` builds the production
        package offline from the deployment's env file. The writer receives
        the descriptor and gate variables and nothing else from it: never
        the NEAR AI key or endpoint, never a database URL."""
        output = self.tmp / "out" / "package.json"
        key_output = self.tmp / "out" / "trusted-key.json"
        env_file = self.tmp / "ingest.env"
        env_file.write_text(
            "\n".join(
                [
                    "# the pilot's env file",
                    "TRACE_COMMONS_GATE_SERVICE=enclave_near_ai",
                    "TRACE_COMMONS_NEAR_AI_MODEL=Qwen/Qwen3.6-35B-A3B-FP8",
                    'TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF="-8.0"',
                    "TRACE_COMMONS_NEAR_AI_API_KEY=secret-near-ai-key",
                    "TRACE_COMMONS_NEAR_AI_BASE_URL=https://example.invalid/v1",
                    "TRACE_COMMONS_EMBEDDER_MODEL_ID='BAAI/bge-large-en-v1.5'",
                    "TRACE_COMMONS_VECTOR_INDEX_DIM=1024",
                    "TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000",
                    "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0",
                    "TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=2.5",
                    "DATABASE_URL=postgres://ingest@db/trace",
                    "",
                ]
            )
        )

        def fake_cargo(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            Path(env["TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT"]).write_text(
                json.dumps({"package": {"bundle_id": _fake_hash("bundle")},
                            "signature": {"package_hash": _fake_hash("package")}})
            )
            Path(env["TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT"]).write_text("{}")

        code = self._main(
            ["package", "--bundle", "production", "--env-file", str(env_file), "--output", str(output),
             "--public-key-output", str(key_output)],
            cargo=fake_cargo,
        )
        self.assertEqual(code, 0, self.stderr.getvalue())
        [(_, step, cargo_args, test_filter, env, _, _)] = self._cargo_calls()
        self.assertEqual((step, cargo_args, test_filter), ("package_write", _INGEST_ARGS, _PACKAGE_WRITER))
        self.assertEqual(
            {key: value for key, value in env.items() if key.startswith("TRACE_COMMONS_")},
            {
                "TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE": "production",
                "TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT": str(output.resolve()),
                "TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT": str(key_output.resolve()),
                "TRACE_COMMONS_NEAR_AI_MODEL": "Qwen/Qwen3.6-35B-A3B-FP8",
                "TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF": "-8.0",
                "TRACE_COMMONS_EMBEDDER_MODEL_ID": "BAAI/bge-large-en-v1.5",
                "TRACE_COMMONS_VECTOR_INDEX_DIM": "1024",
                "TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS": "500000",
                "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS": "0",
                "TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA": "2.5",
            },
        )
        self.assertNotIn("secret-near-ai-key", self.stdout.getvalue() + self.stderr.getvalue())

        bad_line = self.tmp / "bad.env"
        bad_line.write_text("TRACE_COMMONS_NEAR_AI_API_KEY secret-near-ai-key\n")
        for argv, label in (
            (["--bundle", "production"], "package_env_file_required"),
            (["--bundle", "minimal", "--env-file", str(env_file)], "package_env_file_unexpected"),
            (["--bundle", "production", "--env-file", str(self.tmp / "missing.env")], "env_file_unreadable"),
            (["--bundle", "production", "--env-file", str(bad_line)], "env_file_invalid"),
        ):
            with self.subTest(label=label):
                self.calls.clear()
                self.stderr = io.StringIO()
                code = self._main(
                    ["package", *argv, "--output", str(output), "--public-key-output", str(key_output)],
                    cargo=fake_cargo,
                )
                self.assertEqual(code, 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")
                self.assertNotIn("secret-near-ai-key", self.stderr.getvalue())
                self.assertEqual(self._cargo_calls(), [])

        # `run` serves no production bundle: its package comes from `package`.
        self.assertNotIn("production", pipeline.BUNDLES)


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
                    "credit_delta_zero": False,
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
                if seed["credit_delta_zero"]:
                    evidence["credit_delta_zero"] = True
                evidence.update(overrides.get("evidence", {}))
                raw = {
                    "schema": results.SCHEMA,
                    "run_id": env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"],
                    "check_id": _RESTORE_CHECK,
                    "status": "pass",
                    "code_revision_hash": env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"],
                    **_candidate_digests(),
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
    def test_a_zero_delta_seed_passes_and_its_evidence_says_so(self):
        code = self._drill(fingerprint=_UNCREDITED_SEED)
        self.assertEqual(code, 0, self.stderr.getvalue())
        evidence = json.loads((self.run.results_dir / f"{_RESTORE_CHECK}.evidence.json").read_text())
        self.assertIs(evidence["credit_delta_zero"], True)
        self.assertIn("legs_per_run=0 credit_events_per_run=0", self.stdout.getvalue())

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
            ({"fingerprint": {"credit_delta_zero": None}}, "restore_fingerprint_invalid"),
            # A zero-delta seed has no leg, no adapter request, no event.
            ({"fingerprint": {"credit_delta_zero": True}}, "restore_fingerprint_invalid"),
            ({"fingerprint": {**_UNCREDITED_SEED, "completed_settlement_count": 1}}, "restore_fingerprint_invalid"),
            # A zero-delta drill must say so in its evidence.
            ({"fingerprint": _UNCREDITED_SEED, "evidence": {"credit_delta_zero": False}}, "restore_evidence_mismatch"),
            ({"evidence": {"credit_delta_zero": True}}, "restore_evidence_mismatch"),
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


_PRODUCTION_HARNESS_ENV = {
    "TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY": "production",
    "TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH": "/run/signed-package.json",
    "TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH": "/run/trusted-package-key.json",
    "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT": "/run/indexes",
    "TRACE_COMMONS_NEAR_AI_BASE_URL": "https://near-ai.invalid/v1",
    "TRACE_COMMONS_NEAR_AI_API_KEY": "near-ai-test-key",
}


class ProductionHarnessRunnerTests(_RestoreDrillCase):
    """The runners `qualify` uses, in the production mode `promote
    package-checks` starts them in (spec B-D1): the corpus and restore
    evidence must say the production assembly made it exactly when the
    harness variables switched to it, and `run_package_checks` starts the
    four package checks with those variables and the pilot's features."""

    @contextlib.contextmanager
    def _environment(self, cargo):
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(environment, "_invoke", self._invoke))
            stack.enter_context(mock.patch.object(pipeline, "cargo_test", cargo))
            stack.enter_context(contextlib.redirect_stdout(self.stdout))
            stack.enter_context(contextlib.redirect_stderr(self.stderr))
            with pipeline.Environment(self.run, postgres_admin_url=_ADMIN_URL) as env:
                yield env

    def _restore(self, extra_env, **overrides):
        with self._environment(self._fake_cargo(**overrides)) as env:
            return pipeline.run_restore_drill(
                self.run, env, extra_env=extra_env, cargo_args=(*_INGEST_ARGS, *_PROMOTE_FEATURES)
            )

    def test_production_restore_evidence_says_production(self):
        self._restore(_PRODUCTION_HARNESS_ENV, evidence={"harness_assembly": "production"})
        cargo = self._cargo_calls()
        self.assertEqual([call[3] for call in cargo], [_RESTORE_SEED, _RESTORE_RESUME])
        for call, step in zip(cargo, ("seed", "resume")):
            self.assertEqual(call[2], (*_INGEST_ARGS, *_PROMOTE_FEATURES))
            for key, value in _PRODUCTION_HARNESS_ENV.items():
                if key == "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT":
                    # The seed and the resume each open their own index.
                    value = f"/run/indexes/{step}"
                self.assertEqual(call[4][key], value, key)

    def test_reference_restore_opens_no_production_index(self):
        self._restore(None)
        for call in self._cargo_calls():
            self.assertNotIn("TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT", call[4])
            self.assertNotIn("TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY", call[4])

    def test_production_restore_refuses_evidence_without_the_assembly(self):
        with self.assertRaises(errors.ToolingError) as ctx:
            self._restore(_PRODUCTION_HARNESS_ENV)
        self.assertEqual(str(ctx.exception), "restore_evidence_mismatch")

    def test_reference_restore_refuses_production_evidence(self):
        with self.assertRaises(errors.ToolingError) as ctx:
            self._restore(None, evidence={"harness_assembly": "production"})
        self.assertEqual(str(ctx.exception), "restore_evidence_mismatch")

    def _corpus(self, step, extra_env, **harness):
        corpus_path = self.tmp / f"{step}.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture", "beta_fixture"])))
        corpus_run = pipeline.CorpusRun(
            "compatibility",
            None,
            self.tmp / "signed-package.json",
            self.tmp / "trusted-package-key.json",
            [corpus_path],
            [load_direct_corpus_digest(corpus_path)],
        )

        def cargo(run, step_name, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step_name, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            _write_harness_outputs(env, **harness)

        with self._environment(cargo) as env:
            return pipeline.run_corpus_check(
                self.run, env, corpus_run, step=step, extra_env=extra_env, cargo_args=(*_INGEST_ARGS, *_PROMOTE_FEATURES)
            )

    def test_production_corpus_report_and_evidence_say_production(self):
        _, report = self._corpus("corpus_production", _PRODUCTION_HARNESS_ENV)
        self.assertEqual(report["harness_assembly"], "production")
        [call] = self._cargo_calls()
        self.assertEqual(call[2], (*_INGEST_ARGS, *_PROMOTE_FEATURES))
        self.assertEqual(call[4]["TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY"], "production")
        self.assertEqual(call[4]["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"], "pipeline_http_corpus_compatibility")
        self.assertIn("TRACE_COMMONS_PIPELINE_CORPUS_PACKAGE_PATH", call[4])
        self.assertNotIn("TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE", call[4])

    def test_corpus_mode_must_match_the_harness_variables(self):
        cases = {
            "corpus_report_harness_assembly_mismatch:reference report in production": (
                _PRODUCTION_HARNESS_ENV,
                {"report_assembly": "reference"},
            ),
            "corpus_report_harness_assembly_mismatch:production report in reference": (
                None,
                {"report_assembly": "production"},
            ),
            "corpus_evidence_mismatch:production evidence without the assembly": (
                _PRODUCTION_HARNESS_ENV,
                {"evidence_overrides": {"harness_assembly": _DROP}},
            ),
        }
        for index, (case, (extra_env, harness)) in enumerate(cases.items()):
            with self.subTest(case=case):
                with self.assertRaises(errors.ToolingError) as ctx:
                    self._corpus(f"corpus_case_{index}", extra_env, **harness)
                self.assertEqual(str(ctx.exception), case.split(":")[0])

    def test_run_package_checks_starts_the_four_checks_on_the_production_assembly(self):
        bootstrap = self.tmp / "bootstrap-corpus.json"
        holdout = self.tmp / "holdout-corpus.json"
        bootstrap.write_text(json.dumps(_direct_corpus(["alpha_fixture"])))
        holdout.write_text(json.dumps(_direct_corpus(["beta_fixture"], prefix="2")))
        pin = self.tmp / "pin-network.json"
        started = []

        class FakeEnvironment:
            def __init__(self, run, postgres_admin_url=None):
                started.append(("environment", postgres_admin_url))

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def scenario(self, step):
                return ("scenario", step)

        def partitions(run, path, expected_digest):
            started.append(("partitions", Path(path), expected_digest))
            return True, [bootstrap, holdout]

        def database_check(run, scenario, check, *, extra_env=None):
            started.append(("database", scenario, check, extra_env))

        def corpus_check(run, environment_, corpus_run, *, step=None, extra_env=None, cargo_args=None):
            started.append(("corpus", corpus_run, extra_env, cargo_args))

        def restore(run, environment_, *, extra_env=None, cargo_args=None):
            started.append(("restore", extra_env, cargo_args))

        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(pipeline, "Environment", FakeEnvironment))
            stack.enter_context(mock.patch.object(pipeline, "_corpus_partitions", partitions))
            stack.enter_context(mock.patch.object(pipeline, "run_database_check", database_check))
            stack.enter_context(mock.patch.object(pipeline, "run_corpus_check", corpus_check))
            stack.enter_context(mock.patch.object(pipeline, "run_restore_drill", restore))
            pipeline.run_package_checks(
                self.run,
                harness_env=_PRODUCTION_HARNESS_ENV,
                cargo_features=_PROMOTE_FEATURES,
                network_pin=pin,
                postgres_admin_url=_ADMIN_URL,
            )

        def index_root(check_id):
            return {**_PRODUCTION_HARNESS_ENV, "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT": f"/run/indexes/{check_id}"}

        cargo_args = (*_INGEST_ARGS, *_PROMOTE_FEATURES)
        # The HF export runs before any database starts.
        self.assertEqual(started[0], ("partitions", pin, None))
        self.assertEqual(started[1], ("environment", _ADMIN_URL))
        [database] = [event for event in started if event[0] == "database"]
        self.assertEqual(database[2].check_id, "pipeline_bundle_qualification")
        self.assertEqual(database[2].cargo_args[-2:], _PROMOTE_FEATURES)
        self.assertEqual(database[1], ("scenario", "pipeline_bundle_qualification"))
        self.assertEqual(database[3], index_root("pipeline_bundle_qualification"))
        corpus_runs = [event for event in started if event[0] == "corpus"]
        self.assertEqual(
            [event[1].check_id for event in corpus_runs],
            ["pipeline_http_corpus_compatibility", "pipeline_http_corpus_hf_local"],
        )
        for event in corpus_runs:
            corpus_run = event[1]
            self.assertIsNone(corpus_run.bundle)
            self.assertEqual(corpus_run.package_path, Path("/run/signed-package.json"))
            self.assertEqual(corpus_run.key_path, Path("/run/trusted-package-key.json"))
            self.assertEqual(event[2], index_root(corpus_run.check_id))
            self.assertEqual(event[3], cargo_args)
        self.assertEqual(corpus_runs[0][1].partitions, [pipeline.DEFAULT_CORPUS])
        self.assertEqual(corpus_runs[1][1].partitions, [bootstrap, holdout])
        [restore_event] = [event for event in started if event[0] == "restore"]
        self.assertEqual(restore_event[1:], (index_root("pipeline_restore_drill"), cargo_args))


def load_direct_corpus_digest(path):
    return pipeline.load_direct_corpus(path)[1]


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
# The checks that test the candidate package, and so carry its digests
# (P5-D15); every other required check carries none.
_CANDIDATE_CHECKS = (
    "pipeline_bundle_qualification",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
    "pipeline_restore_drill",
)


def _rust_check_list(name):
    """The check ids of the Rust constant `name`, read out of the source."""
    source = _PROMOTION_SOURCE.read_text()
    match = re.search(rf"pub const {name}: &\[&str\] = &\[(.*?)\];", source, re.S)
    if match is None:
        raise AssertionError(f"{name} not found")
    return re.findall(r'"([a-z0-9_]+)"', match.group(1))


def _promotion_required_checks():
    """`PROMOTION_REQUIRED_CHECKS`, read out of the Rust source."""
    return _rust_check_list("PROMOTION_REQUIRED_CHECKS")


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
        **(_candidate_digests() if digests else _NO_DIGESTS),
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
        # The Rust constant that `evaluate_promotion` reads for its one-run
        # rule (review round 1 of #1240, point 5) holds the same three ids:
        # a check that `qualify` produces and Rust called promotion-only
        # would escape the rule.
        promotion_only = _rust_check_list("PROMOTION_ONLY_CHECKS")
        self.assertEqual(len(promotion_only), len(set(promotion_only)))
        self.assertEqual(frozenset(promotion_only), _PROMOTION_ONLY)
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
        self.assertEqual(len(database_ids), 15)
        self.assertEqual(len(set(database_ids)), 15)
        self.assertEqual(
            checks.REQUIRED_CHECK_IDS,
            frozenset(database_ids) | set(checks.REQUIRED_CORPUS_CHECK_IDS) | {checks.RESTORE_CHECK_ID},
        )
        for check in checks.REQUIRED_DATABASE_CHECKS:
            with self.subTest(check=check.check_id):
                self.assertRegex(check.check_id, r"^[a-z0-9_]{1,64}$")
                self.assertIn(check.database, ("upgrade", "runtime", "pilot"))
                self.assertEqual(check.digests, check.check_id in _CANDIDATE_CHECKS)

    def test_rust_package_checks_are_the_checks_that_require_digests(self):
        """`evaluate_promotion` enforces which checks name the package
        (`PROMOTION_PACKAGE_CHECKS`) and this tooling enforces the same: the
        four `qualify` produces with `digests_required`, and (spec
        2026-10-08, A-D12) the three promotion-only checks, which run against
        the production assembly and name its package. The lists are one
        set, `checks.PROMOTION_PACKAGE_CHECK_IDS`."""
        rust = _rust_check_list("PROMOTION_PACKAGE_CHECKS")
        self.assertEqual(len(rust), len(set(rust)), "no duplicate package check id")
        self.assertEqual(set(rust), set(_CANDIDATE_CHECKS) | _PROMOTION_ONLY)
        self.assertEqual(len(rust), 7)
        self.assertTrue(set(rust).issubset(_promotion_required_checks()))
        self.assertEqual(frozenset(rust), checks.PROMOTION_PACKAGE_CHECK_IDS)
        self.assertEqual(checks.PROMOTION_ONLY_CHECK_IDS, _PROMOTION_ONLY)
        self.assertEqual(
            checks.PROMOTION_PACKAGE_CHECK_IDS - checks.PROMOTION_ONLY_CHECK_IDS,
            {spec.check_id for spec in checks.required_specs().values() if spec.digests_required},
        )

    def test_only_candidate_checks_require_package_digests(self):
        """P5-D15: a qualification run names exactly one package. The four
        checks that test the candidate carry its three digests, and every
        mechanics check carries none, so `evaluate_promotion` finds one
        package among the results. A fifth check that asked for digests
        would name a test bundle again."""
        required = {spec.check_id for spec in checks.required_specs().values() if spec.digests_required}
        self.assertEqual(required, set(_CANDIDATE_CHECKS))
        self.assertEqual(len(checks.required_specs()), 19)
        # The database checks' own rows agree with `required_specs`.
        self.assertEqual(
            {check.check_id for check in checks.REQUIRED_DATABASE_CHECKS if check.digests},
            {"pipeline_bundle_qualification"},
        )

    def test_the_activation_checks_are_required(self):
        """PR 5's three checks (containment, rollback, and the legacy drain
        report) are required in both lists, each run by the one exact test
        that emits it. They are mechanics checks: none names a package, so a
        qualification run still names exactly one."""
        expected = {
            "pipeline_activation_containment": (
                checks._INGEST_BIN,
                checks._HTTP_TESTS + "containment_refuses_new_receipts_and_keeps_pending_work",
                "pilot",
            ),
            "pipeline_activation_rollback": (
                checks._RUNTIME_SUITE,
                "rollback_selects_an_earlier_bundle_for_new_runs_only",
                "runtime",
            ),
            "pipeline_legacy_drain": (
                checks._INGEST_BIN,
                "tests::pipeline_activation_pg_tests::the_legacy_drain_report_counts_real_pending_work_and_reaches_zero",
                "pilot",
            ),
        }
        promotion = _promotion_required_checks()
        rows = {check.check_id: check for check in checks.REQUIRED_DATABASE_CHECKS}
        specs = checks.required_specs()
        for check_id, (cargo_args, test_name, database) in expected.items():
            with self.subTest(check=check_id):
                self.assertIn(check_id, checks.REQUIRED_CHECK_IDS)
                self.assertIn(check_id, promotion)
                self.assertNotIn(check_id, _PROMOTION_ONLY)
                self.assertNotIn(check_id, _CANDIDATE_CHECKS)
                self.assertFalse(specs[check_id].digests_required)
                row = rows[check_id]
                self.assertEqual(
                    (row.cargo_args, row.test_name, row.database, row.ignored, row.digests),
                    (cargo_args, test_name, database, False, False),
                )
        # The 15 database rows, the three corpus checks, and the restore
        # drill: 19 required results, four of them naming the one package.
        self.assertEqual(len(checks.REQUIRED_CHECK_IDS), 19)
        self.assertEqual(sum(spec.digests_required for spec in specs.values()), 4)


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
        # Check ids whose result names another package than the candidate's,
        # and check ids whose result names a package (True) or none (False)
        # whatever their row says.
        self.other_package = set()
        self.digest_override = {}
        # The corpus harness's result names the report's package (True) or
        # none (False) for a check id here; the others follow the default.
        self.harness_digests = {}
        # The attestation writer's fake: check ids it writes no file for, a
        # key id it writes instead of the one it was given, per-check fields
        # it replaces, and whether the step itself fails.
        self.attestation_skip = set()
        self.attestation_key_id = None
        self.attestation_edits = {}
        self.attestation_fails = False
        # Called with the restore resume's environment once it has written
        # its result, the last check `qualify` runs: a late change to what an
        # earlier check left behind.
        self.after_last_check = None
        # Called with the signing step's environment before the fake signer
        # reads the results (a change between the check and the signer), and
        # after it wrote its files (a change between the signer and the tool's
        # final checks).
        self.before_signing = None
        self.during_signing = None
        # Whether the fake signer refuses result files off the list it is given.
        self.signer_honors_ids = True

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
                if test_filter == _RESTORE_RESUME and self.after_last_check is not None:
                    self.after_last_check(env)
                return
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            if test_filter == _HARNESS:
                _write_harness_outputs(
                    env,
                    emit=self.harness_emits,
                    digests=self.harness_digests.get(env["TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID"]),
                )
            elif test_filter == _ATTESTATION_WRITER:
                if self.attestation_fails:
                    raise errors.StepFailed(step, 101, run.log_path(step))
                if self.before_signing is not None:
                    self.before_signing(env)
                self._write_attestations(env)
                if self.during_signing is not None:
                    self.during_signing(env)
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
                    named = self.digest_override.get(check.check_id, check.digests)
                    other = {"package_hash": _fake_hash("other-package")} if check.check_id in self.other_package else {}
                    _emit_check(env, check.check_id, digests=named, **other)

        return fake_cargo_test

    def _write_attestations(self, env):
        """What `pipeline_check_attestations_write` leaves behind: for each
        result file, an attestation that wraps the result as it is, carries
        the evidence's two digests when it holds both, and names the key id
        it was given. The signature bytes are fake: only the Rust writer
        verifies them."""
        result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
        output_dir = Path(env.get("TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR", result_dir))
        accepted = set(env["TRACE_COMMONS_PIPELINE_CHECK_IDS"].split(",")) if self.signer_honors_ids else None
        result_paths = sorted(result_dir.glob("*.result.json"))
        if accepted is not None:
            # The real signer refuses a result file that is not on the list,
            # and a listed check without one, before it signs anything.
            found = {json.loads(path.read_bytes())["check_id"] for path in result_paths}
            if found != accepted:
                raise errors.StepFailed("check_attestations", 101, self.run.log_path("check_attestations"))
        for result_path in result_paths:
            result = json.loads(result_path.read_bytes())
            check_id = result["check_id"]
            if check_id in self.attestation_skip:
                continue
            evidence = json.loads((result_dir / f"{check_id}.evidence.json").read_bytes())
            both = isinstance(evidence, dict) and "corpus_digest" in evidence and "input_digest" in evidence
            attestation = {
                "schema": "trace_commons.pipeline_check_attestation.v1",
                "result": result,
                "maximum_age_seconds": int(env["TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS"]),
                "corpus_digest": evidence["corpus_digest"] if both else None,
                "input_digest": evidence["input_digest"] if both else None,
                "signature": {
                    "algorithm": "Ed25519",
                    "key_id": self.attestation_key_id or env["TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID"],
                    "attestation_hash": _fake_hash(f"attestation:{check_id}"),
                    "signature_base64url": "A" * 86,
                },
            }
            attestation.update(self.attestation_edits.get(check_id, {}))
            (output_dir / f"{check_id}.attestation.json").write_text(json.dumps(attestation))

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

    def test_qualify_names_one_package_on_exactly_the_four_candidate_checks(self):
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        loaded = results.load_results(self.run)
        named = {check_id for check_id, result in loaded.items() if result.package_hash is not None}
        self.assertEqual(named, set(_CANDIDATE_CHECKS))
        self.assertEqual(
            {(result.package_hash, result.configuration_digest, result.dependency_digest) for result in loaded.values()}
            - {(None, None, None)},
            {tuple(_candidate_digests().values())},
        )
        self.assertEqual(
            (loaded["pipeline_http_corpus_minimal"].package_hash, loaded["pipeline_crash_matrix"].package_hash),
            (None, None),
        )
        # The report's checks say the same.
        value = json.loads(self.report_path.read_text())
        self.assertEqual(
            {item["check_id"] for item in value["checks"] if item["package_hash"] is not None},
            set(_CANDIDATE_CHECKS),
        )

    def test_qualify_refuses_a_candidate_check_that_names_another_package(self):
        self.other_package = {"pipeline_bundle_qualification"}
        self.assertEqual(self._qualify(), 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: qualification_evidence_mixed_package")
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", "qualification_evidence_mixed_package"))
        self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())

    def test_qualify_refuses_a_mechanics_check_that_names_a_package(self):
        self.digest_override = {"pipeline_crash_matrix": True}
        self.assertEqual(self._qualify(), 1)
        label = "pipeline_check_digests_unexpected:pipeline_crash_matrix"
        self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", label))

        with self.subTest("the minimal corpus check names no package either"):
            self._fresh_run()
            self.digest_override = {}
            self.harness_digests = {"pipeline_http_corpus_minimal": True}
            self.assertEqual(self._qualify(), 1)
            self.assertEqual(
                self.stderr.getvalue().strip(),
                "PipelineFailure: pipeline_check_digests_unexpected:pipeline_http_corpus_minimal",
            )

    def test_qualify_refuses_a_candidate_check_without_its_digests(self):
        self.digest_override = {"pipeline_bundle_qualification": False}
        self.assertEqual(self._qualify(), 1)
        self.assertEqual(
            self.stderr.getvalue().strip(), "PipelineFailure: check_result_digest_missing:pipeline_bundle_qualification"
        )

        with self.subTest("a compatibility corpus result without its digests"):
            self._fresh_run()
            self.digest_override = {}
            self.harness_digests = {"pipeline_http_corpus_compatibility": False}
            self.assertEqual(self._qualify(), 1)
            self.assertEqual(
                self.stderr.getvalue().strip(),
                "PipelineFailure: check_result_digest_missing:pipeline_http_corpus_compatibility",
            )

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
        self.assertEqual(len(databases), 15)

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
        self.assertEqual(lines[0], f"PipelineQualificationOK: report={self.report_path.name} checks=19")
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

    def test_a_pass_report_names_one_package_on_the_candidate_checks_only(self):
        """The pass branch of the report validator applies the rules of the
        results (P5-D15), for a report that `catalog.py` reads from disk:
        one package among the checks, digests exactly on the four candidate
        checks. Each direction is refused, and the correct report passes."""
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        value = json.loads(self.report_path.read_text())
        report_module.validate_qualification_report(value)  # the correct one passes
        named = {
            item["check_id"] for item in value["checks"] if item["package_hash"] is not None
        }
        self.assertEqual(named, set(_CANDIDATE_CHECKS))

        def changed(check_id, **fields):
            checks_ = [
                {**item, **fields} if item["check_id"] == check_id else item for item in value["checks"]
            ]
            report = {**value, "checks": checks_}
            report["evidence_hash"] = _digest(
                results.canonical({"inputs": report["inputs"], "checks": report["checks"]})
            )
            return report

        cases = (
            (
                "two packages",
                changed("pipeline_http_corpus_hf_local", package_hash=_fake_hash("another-package")),
                "qualification_evidence_mixed_package",
            ),
            (
                "a mechanics check with a digest",
                changed("pipeline_crash_matrix", **_candidate_digests()),
                "pipeline_check_digests_unexpected:pipeline_crash_matrix",
            ),
            (
                "a candidate check without its digests",
                changed("pipeline_restore_drill", **_NO_DIGESTS),
                "check_result_digest_missing:pipeline_restore_drill",
            ),
        )
        for label, report, expected in cases:
            with self.subTest(label):
                with self.assertRaises(errors.ToolingError) as ctx:
                    report_module.validate_qualification_report(report)
                self.assertEqual(str(ctx.exception), expected)

        with self.subTest("the catalog refuses a pass report that breaks a rule"):
            self.report_path.write_text(json.dumps(cases[1][1]))
            with self.assertRaises(errors.ToolingError) as ctx:
                catalog.update_catalog(self.catalog_path, self.report_path)
            self.assertEqual(str(ctx.exception), cases[1][2])
            self.assertFalse(self.catalog_path.exists())

    def test_a_v1_report_without_the_attestation_keys_validates_and_is_archived(self):
        """The report keeps schema `v1`. A `v1` report without `attested` and
        `attestation_count` (here: this tool's own pass report with the two
        keys taken out) validates, reads as unsigned (`attested: false`,
        `attestation_count: 0`), and `qualify --archive` archives it. A report
        with one of the two keys must have both, and they must agree. That a
        key-less report is valid does not make every report of `main`'s tool
        valid: see `test_a_pass_report_shaped_as_mains_tool_writes_it_...`."""
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        written = json.loads(self.report_path.read_text())
        self.assertEqual((written["attested"], written["attestation_count"]), (False, 0), "the tool writes both keys")
        main_style = {key: value for key, value in written.items() if key not in ("attested", "attestation_count")}
        self.assertEqual(main_style["schema"], "trace_commons.pipeline_qualification_report.v1")
        report_module.validate_qualification_report(main_style)
        self.assertEqual(report_module.report_attestation(main_style), (False, 0))
        self.assertEqual(report_module.report_attestation(written), (False, 0))
        archived_source = self.tmp / "main-style-report.json"
        archived_source.write_text(json.dumps(main_style))
        catalog.update_catalog(self.catalog_path, archived_source)
        [entry] = json.loads(self.catalog_path.read_text())["qualifications"]
        self.assertEqual(entry["status"], "pass")

        # A signed pass report, then each way a report can contradict itself.
        self._fresh_run()
        self.assertEqual(self._qualify("--signing-key", str(self._key()), "--signing-key-id", "ci_key"), 0, self.stderr.getvalue())
        signed = json.loads(self.report_path.read_text())
        self.assertEqual((signed["attested"], signed["attestation_count"]), (True, 19))
        report_module.validate_qualification_report(signed)
        self.assertEqual(report_module.report_attestation(signed), (True, 19))
        failed = {**signed, "status": "fail", "failure": "check_attestation_invalid"}
        cases = {
            # One key alone, even when it agrees with what the other would read as.
            "only attested, false": {**main_style, "attested": False},
            "only attestation_count, zero": {**main_style, "attestation_count": 0},
            "only attested": {key: value for key, value in signed.items() if key != "attestation_count"},
            "only attestation_count": {key: value for key, value in signed.items() if key != "attested"},
            "attested with no attestation": {**signed, "attestation_count": 0},
            "attestations that are not attested": {**signed, "attested": False},
            "fewer attestations than checks": {**signed, "attestation_count": 18},
            "more attestations than checks": {**signed, "attestation_count": 20},
            "a count that is not a number": {**signed, "attestation_count": "19"},
            "an attested failed report": failed,
        }
        for label, report in cases.items():
            with self.subTest(label):
                with self.assertRaises(errors.ToolingError) as ctx:
                    report_module.validate_qualification_report(report)
                self.assertEqual(str(ctx.exception), "qualification_report_invalid")
        # An unattested failed report is valid, and any other key is not.
        report_module.validate_qualification_report({**failed, "attested": False, "attestation_count": 0})
        with self.assertRaises(errors.ToolingError):
            report_module.validate_qualification_report({**main_style, "unknown_key": 1})

    def test_a_pass_report_shaped_as_mains_tool_writes_it_is_refused_and_its_fail_report_validates(self):
        """A `v1` report without the two keys validates and reads as unsigned.
        That holds for a FAIL report of `main`'s tool. It does not hold for a
        PASS report of `main`'s tool: its evidence names more than one package
        (the minimal corpus check carries a package, and so does each
        mechanics check: a run names many), which `_require_one_package`
        refuses, because such a result set could never back a promotion."""
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        written = json.loads(self.report_path.read_text())

        def main_shaped(candidate_only=False, **changes):
            """`written` as `main`'s tool writes a report: no attestation keys,
            and each check (every one, or only the four candidate checks)
            naming a package of its own."""
            shaped_checks = []
            for item in written["checks"]:
                own = candidate_only is False or item["check_id"] in _CANDIDATE_CHECKS
                shaped_checks.append(
                    {
                        **item,
                        "package_hash": _fake_hash(f"package:{item['check_id']}") if own else None,
                        "configuration_digest": _fake_hash(f"configuration:{item['check_id']}") if own else None,
                        "dependency_digest": _fake_hash(f"dependency:{item['check_id']}") if own else None,
                    }
                )
            shaped = {key: value for key, value in written.items() if key not in ("attested", "attestation_count")}
            shaped.update(changes)
            shaped["checks"] = shaped_checks
            shaped["evidence_hash"] = _digest(results.canonical({"inputs": shaped["inputs"], "checks": shaped_checks}))
            return shaped

        everything_named = main_shaped()
        packages = {item["package_hash"] for item in everything_named["checks"]}
        self.assertGreater(len(packages), 11, "a run of `main`'s tool names many packages")
        first_mechanics = next(item["check_id"] for item in everything_named["checks"] if item["check_id"] not in _CANDIDATE_CHECKS)
        for label, report, expected in (
            (
                "each check names its own package",
                everything_named,
                f"pipeline_check_digests_unexpected:{first_mechanics}",
            ),
            (
                "the four candidate checks name four packages",
                main_shaped(candidate_only=True),
                "qualification_evidence_mixed_package",
            ),
        ):
            with self.subTest(label):
                self.assertNotIn("attested", report)
                with self.assertRaises(errors.ToolingError) as ctx:
                    report_module.validate_qualification_report(report)
                self.assertEqual(str(ctx.exception), expected)

        failed = main_shaped(status="fail", failure="check_result_failed:pipeline_crash_matrix")
        report_module.validate_qualification_report(failed)  # a fail report of the same shape validates
        self.assertEqual(report_module.report_attestation(failed), (False, 0))
        failed_path = self.tmp / "main-style-fail-report.json"
        failed_path.write_text(json.dumps(failed))
        catalog.update_catalog(self.catalog_path, failed_path)

    def _key(self):
        key = self.tmp / "tcsecretkey-report-test.pk8"
        key.write_bytes(b"a disposable test key, not a real one")
        return key

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


# ---------------------------------------------------------------------------
# PR 5, Task 8: signed check results (`qualify --signing-key`, `keygen`,
# `revision`).
# ---------------------------------------------------------------------------

_ATTESTATION_VARS = (
    "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR",
    "TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR",
    "TRACE_COMMONS_PIPELINE_CHECK_IDS",
    "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_PATH",
    "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID",
    "TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS",
)
_KEYGEN_VARS = (
    "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT",
    "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_KEY_ID",
    "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT",
)


class SignedQualifyTests(_QualifyCase):
    KEY_NAME = "tcsecretkey-7f3a91.pk8"

    def setUp(self):
        super().setUp()
        self.key = self.tmp / self.KEY_NAME
        self.key.write_bytes(b"a disposable test key, not a real one")

    def _signed(self, *extra):
        return self._qualify("--signing-key", str(self.key), "--signing-key-id", "ci_key", *extra)

    def _attestation_calls(self):
        return [call for call in self._cargo_calls() if call[3] == _ATTESTATION_WRITER]

    def _attestation_files(self):
        return sorted(path.name for path in self.run.results_dir.glob("*.attestation.json"))

    def test_qualify_with_a_signing_key_runs_the_attestation_step(self):
        # Without the key: no attestation step, no files, an unattested report.
        self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
        unsigned_calls = len(self._cargo_calls())
        self.assertEqual(self._attestation_calls(), [])
        self.assertEqual(self._attestation_files(), [])
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
        report_module.validate_qualification_report(value)
        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(lines[0], f"PipelineQualificationOK: report={self.report_path.name} checks=19")

        # With it: one more cargo call, the writer, after every check.
        self._fresh_run()
        self.assertEqual(self._signed(), 0, self.stderr.getvalue())
        calls = self._cargo_calls()
        self.assertEqual(len(calls), unsigned_calls + 1)
        [attestation_call] = self._attestation_calls()
        self.assertIs(calls[-1], attestation_call, "the writer runs after every check")
        _, step, cargo_args, test_filter, env, exact, ignored = attestation_call
        self.assertRegex(step, r"^[a-z0-9_]{1,64}$")
        self.assertEqual((cargo_args, test_filter, exact, ignored), (_INGEST_ARGS, _ATTESTATION_WRITER, True, True))
        self.assertEqual({key for key in env if key.startswith("TRACE_COMMONS_")}, set(_ATTESTATION_VARS))
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"], str(self.run.results_dir))
        # The signer writes into a staging directory of the run directory, and
        # signs exactly the required checks.
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR"], str(self.run.run_dir / "attestations-staging"))
        self.assertEqual(
            env["TRACE_COMMONS_PIPELINE_CHECK_IDS"].split(","), sorted(checks.REQUIRED_CHECK_IDS)
        )
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_PATH"], str(self.key.resolve()))
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID"], "ci_key")
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS"], "86400")
        self.assertEqual(
            self._attestation_files(), sorted(f"{check_id}.attestation.json" for check_id in checks.REQUIRED_CHECK_IDS)
        )
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["attested"], value["attestation_count"]), ("pass", True, 19))
        report_module.validate_qualification_report(value)
        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(lines[0], f"PipelineQualificationOK: report={self.report_path.name} checks=19 attested=19")
        self.assertEqual(self.stderr.getvalue(), "")
        # Only the three corpus checks carry the evidence's two digests.
        carried = {}
        for check_id in checks.REQUIRED_CHECK_IDS:
            raw = json.loads((self.run.results_dir / f"{check_id}.attestation.json").read_text())
            carried[check_id] = (raw["corpus_digest"], raw["input_digest"])
        self.assertEqual(
            {check_id for check_id, pair in carried.items() if pair != (None, None)},
            set(checks.REQUIRED_CORPUS_CHECK_IDS),
        )

        # The age flag reaches the writer.
        self._fresh_run()
        self.assertEqual(self._signed("--evidence-max-age-seconds", "3600"), 0, self.stderr.getvalue())
        [attestation_call] = self._attestation_calls()
        self.assertEqual(attestation_call[4]["TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS"], "3600")

    def test_the_attestation_step_never_runs_for_a_result_set_that_does_not_pass(self):
        """The step runs only after `require_current_pass_results` passed: a
        missing, foreign, or package-breaking result stops the run first, so
        nothing is signed and the report says so."""
        cases = {
            "a missing result": ({"silent": {"pipeline_payout_recovery"}}, "check_result_missing:pipeline_payout_recovery"),
            "a result from another run": ({"foreign": {"pipeline_receipt_replay_exact"}}, "check_result_foreign_run"),
            "a mechanics result that names a package": (
                {"digest_override": {"pipeline_crash_matrix": True}},
                "pipeline_check_digests_unexpected:pipeline_crash_matrix",
            ),
            "a candidate result without its digests": (
                {"digest_override": {"pipeline_bundle_qualification": False}},
                "check_result_digest_missing:pipeline_bundle_qualification",
            ),
        }
        for label, (knobs, failure) in cases.items():
            with self.subTest(label):
                self._fresh_run()
                self.silent, self.foreign, self.digest_override = set(), set(), {}
                for name, value in knobs.items():
                    setattr(self, name, value)
                self.assertEqual(self._signed(), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {failure}")
                self.assertEqual(self._attestation_calls(), [])
                self.assertEqual(self._attestation_files(), [])
                value = json.loads(self.report_path.read_text())
                self.assertEqual((value["status"], value["failure"]), ("fail", failure))
                self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
                self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())

    def test_a_result_that_changes_after_its_own_check_is_never_signed(self):
        """Each check requires its own result when it runs, and the whole set
        is required again once every check is done. A result that a later step
        removed or changed after its own check passed is caught by that last
        requirement, before anything is signed."""

        def remove(check_id):
            def tamper(env):
                result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
                (result_dir / f"{check_id}.result.json").unlink()
                (result_dir / f"{check_id}.evidence.json").unlink()

            return tamper

        def fail(check_id):
            def tamper(env):
                path = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"]) / f"{check_id}.result.json"
                path.write_text(json.dumps({**json.loads(path.read_text()), "status": "fail"}))

            return tamper

        cases = (
            ("a result removed late", remove("pipeline_crash_matrix"), "check_result_missing:pipeline_crash_matrix"),
            ("a result failed late", fail("pipeline_crash_matrix"), "check_result_failed:pipeline_crash_matrix"),
        )
        for label, tamper, failure in cases:
            with self.subTest(label):
                self._fresh_run()
                self.after_last_check = tamper
                self.assertEqual(self._signed(), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {failure}")
                self.assertEqual(self._attestation_calls(), [], "nothing was signed")
                self.assertEqual(self._attestation_files(), [])
                value = json.loads(self.report_path.read_text())
                self.assertEqual((value["status"], value["failure"]), ("fail", failure))
                self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
        self.after_last_check = None

    def _assert_no_attestation_anywhere(self):
        self.assertEqual(
            sorted(path.name for path in self.run.run_dir.rglob("*.attestation.json")), [],
            "a failed run leaves no attestation file anywhere in its directory",
        )
        self.assertFalse((self.run.run_dir / "attestations-staging").exists(), "no staging directory is left")

    def _assert_failed_unattested(self, failure):
        value = json.loads(self.report_path.read_text())
        self.assertEqual((value["status"], value["failure"]), ("fail", failure))
        self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
        self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())

    def test_a_signed_run_whose_tree_changed_leaves_no_attestation(self):
        """The tree check of PR 4 exists so that evidence from a mixed tree is
        never credited to the starting revision. Attestations name that
        revision and the server accepts them when it is deployed, so a run
        that fails the tree check leaves none, whenever the tree changed."""
        # Changed during the checks: the tree is checked before anything is
        # signed, so nothing is signed.
        self.tree_edited = True
        self.assertEqual(self._signed("--archive"), 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: code_revision_changed")
        self.assertEqual(self._attestation_calls(), [], "the tree is checked before the signing step")
        self._assert_no_attestation_anywhere()
        self._assert_failed_unattested("code_revision_changed")
        self.assertEqual(self.tree_hash_calls, 1)
        self.assertFalse(self.catalog_path.exists(), "nothing is archived")

    def test_a_tree_that_changes_while_the_results_are_signed_leaves_no_attestation(self):
        def edit_the_tree(env):
            self.tree_edited = True

        self.during_signing = edit_the_tree
        self.assertEqual(self._signed("--archive"), 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: code_revision_changed")
        self.assertEqual(len(self._attestation_calls()), 1, "the results were signed, then the second check failed")
        self._assert_no_attestation_anywhere()
        self._assert_failed_unattested("code_revision_changed")
        self.assertEqual(self.tree_hash_calls, 2, "once before the signing step, once after it")
        self.assertFalse(self.catalog_path.exists(), "nothing is archived")

    def test_a_failure_while_the_attestations_are_published_leaves_none(self):
        published = []
        real = pipeline._publish_file

        def failing(source, destination):
            published.append(destination.name)
            if len(published) == 5:
                raise OSError("disk full")
            real(source, destination)

        with mock.patch.object(pipeline, "_publish_file", failing):
            self.assertEqual(self._signed("--archive"), 1)
        self.assertEqual(len(published), 5, "four files were already in place when the fifth failed")
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: check_attestation_publish_failed")
        self._assert_no_attestation_anywhere()
        self._assert_failed_unattested("check_attestation_publish_failed")
        self.assertFalse(self.catalog_path.exists(), "nothing is archived")

    def test_a_signed_run_that_passes_leaves_its_files_in_place_and_no_staging_directory(self):
        self.assertEqual(self._signed(), 0, self.stderr.getvalue())
        self.assertEqual(
            sorted(path.name for path in self.run.results_dir.glob("*.attestation.json")),
            sorted(f"{check_id}.attestation.json" for check_id in checks.REQUIRED_CHECK_IDS),
        )
        self.assertEqual(
            sorted(path.name for path in self.run.run_dir.rglob("*.attestation.json")),
            sorted(path.name for path in self.run.results_dir.glob("*.attestation.json")),
            "the only attestation files are the final ones",
        )
        self.assertFalse((self.run.run_dir / "attestations-staging").exists())
        self.assertEqual(self.tree_hash_calls, 2, "checked before the signing step and again after it")

    def test_a_failed_run_that_cannot_remove_its_attestations_says_so(self):
        """`discard_attestations` looks again after it removed: anything left
        (the staging directory, or an attestation file) is
        `check_attestation_discard_failed`, shown on the terminal and carried
        by the failure report beside the original label."""

        class ShutilWithoutRmtree:
            def __getattr__(self, name):
                return getattr(shutil, name)

            @staticmethod
            def rmtree(path, ignore_errors=False):
                return None

        real_unlink = Path.unlink

        def stubborn_unlink(path, missing_ok=False):
            if path.name.endswith(".attestation.json"):
                raise PermissionError("denied")
            return real_unlink(path, missing_ok=missing_ok)

        def edit_the_tree(env):
            self.tree_edited = True

        def conflicting_archive(*args, **kwargs):
            raise errors.ToolingError("immutable_record_conflict")

        discard_failed = "check_attestation_discard_failed"
        cases = (
            # The tree changes while the results are signed: the files are in
            # the staging directory, which cannot be removed.
            ("a staging directory that stays", "code_revision_changed", {"during_signing": edit_the_tree},
             [("shutil", ShutilWithoutRmtree())], True),
            # The archive fails after the files were published: they cannot be
            # unlinked.
            ("published files that stay", "immutable_record_conflict", {},
             [("update_catalog", conflicting_archive)], False),
        )
        for label, original, hooks, replaced, staged in cases:
            with self.subTest(label):
                self._fresh_run()
                self.during_signing = hooks.get("during_signing")
                self.tree_edited = False
                with contextlib.ExitStack() as stack:
                    for name, replacement in replaced:
                        stack.enter_context(mock.patch.object(pipeline, name, replacement))
                    stack.enter_context(mock.patch.object(Path, "unlink", stubborn_unlink))
                    self.assertEqual(self._signed("--archive"), 1)
                lines = self.stderr.getvalue().splitlines()
                self.assertIn(f"PipelineFailure: {original}", lines, "the original label is still shown")
                self.assertIn(f"PipelineFailure: {discard_failed}", lines)
                value = json.loads(self.report_path.read_text())
                self.assertEqual((value["status"], value["failure"]), ("fail", f"{original}.{discard_failed}"))
                self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
                report_module.validate_qualification_report(value)
                # The premise: what could not be removed is really still there.
                self.assertEqual((self.run.run_dir / "attestations-staging").exists(), staged)
                self.assertEqual(
                    bool(list(self.run.run_dir.rglob("*.attestation.json"))), True, "an attestation file remains"
                )
        self.during_signing = None

    def test_a_failure_that_cannot_write_its_failed_report_leaves_no_pass_report(self):
        """An attested pass report is written before the files are published
        and the archive is made. If a later step fails and the failed report
        cannot be written either, this run's pass report must not stay, in
        either place it was written."""
        real_write = pipeline.write_report

        def pass_reports_only(*args, **kwargs):
            if kwargs.get("failure") is not None:
                raise OSError("disk full")
            return real_write(*args, **kwargs)

        published = []
        real_publish = pipeline._publish_file

        def failing_publish(source, destination):
            published.append(destination.name)
            if len(published) == 5:
                raise OSError("disk full")
            real_publish(source, destination)

        def conflicting_archive(*args, **kwargs):
            raise errors.ToolingError("immutable_record_conflict")

        cases = (
            ("the publish fails", "check_attestation_publish_failed", True, ("_publish_file", failing_publish)),
            ("the archive fails", "immutable_record_conflict", True, ("update_catalog", conflicting_archive)),
            ("the archive fails without a signing key", "immutable_record_conflict", False,
             ("update_catalog", conflicting_archive)),
        )
        for label, original, signed, (name, replacement) in cases:
            with self.subTest(label):
                self._fresh_run()
                published.clear()
                with mock.patch.object(pipeline, name, replacement), mock.patch.object(
                    pipeline, "write_report", pass_reports_only
                ):
                    code = self._signed("--archive") if signed else self._qualify("--archive")
                self.assertEqual(code, 1)
                self.assertIn(f"PipelineFailure: {original}", self.stderr.getvalue())
                self.assertFalse(self.report_path.exists(), "no pass report in the latest-report place")
                self.assertFalse((self.run.run_dir / "qualification-report.json").exists(), "none in the run directory")
                self._assert_no_attestation_anywhere()

    def test_a_result_file_changed_before_the_signer_reads_it_is_not_signed(self):
        """The attestations are compared with the results that
        `require_current_pass_results` accepted, not with the files on disk, so
        a result file changed between the check and the signer cannot be signed
        and pass."""

        def changing(**fields):
            def tamper(env):
                path = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"]) / "pipeline_crash_matrix.result.json"
                path.write_text(json.dumps({**json.loads(path.read_text()), **fields}))

            return tamper

        for label, tamper in (
            ("a status changed", changing(status="fail")),
            ("a time moved", changing(observed_at=_iso(datetime.now(timezone.utc) + timedelta(seconds=30)))),
            ("a safe blocker added", changing(safe_blockers=["extra_blocker"])),
        ):
            with self.subTest(label):
                self._fresh_run()
                self.before_signing = tamper
                self.assertEqual(self._signed(), 1)
                self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: check_attestation_invalid")
                self._assert_no_attestation_anywhere()
                self._assert_failed_unattested("check_attestation_invalid")
        self.before_signing = None

    def test_a_failed_attestation_step_fails_the_qualification(self):
        cases = {
            "no file for one result": ({"attestation_skip": {"pipeline_crash_matrix"}}, 1, "check_attestation_count_mismatch"),
            "a file for another key": ({"attestation_key_id": "other_key"}, 1, "check_attestation_invalid"),
            "another maximum age": (
                {"attestation_edits": {"pipeline_crash_matrix": {"maximum_age_seconds": 5}}},
                1,
                "check_attestation_invalid",
            ),
            "another schema": (
                {"attestation_edits": {"pipeline_crash_matrix": {"schema": "trace_commons.other.v1"}}},
                1,
                "check_attestation_invalid",
            ),
            "a digest the evidence does not hold": (
                {"attestation_edits": {"pipeline_crash_matrix": {"corpus_digest": _fake_hash("c")}}},
                1,
                "check_attestation_invalid",
            ),
            "a digest the evidence holds, left out": (
                {"attestation_edits": {"pipeline_http_corpus_minimal": {"input_digest": None}}},
                1,
                "check_attestation_invalid",
            ),
            "a result that is not the result file": (
                {"attestation_edits": {"pipeline_crash_matrix": {"result": {"check_id": "pipeline_crash_matrix"}}}},
                1,
                "check_attestation_invalid",
            ),
            "an unknown field": (
                {"attestation_edits": {"pipeline_crash_matrix": {"note": "x"}}},
                1,
                "check_attestation_invalid",
            ),
            "a step that fails": ({"attestation_fails": True}, 101, "step_failed:check_attestations"),
        }
        for label, (knobs, exit_code, failure) in cases.items():
            with self.subTest(label):
                self._fresh_run()
                self.attestation_skip, self.attestation_key_id = set(), None
                self.attestation_edits, self.attestation_fails = {}, False
                for name, value in knobs.items():
                    setattr(self, name, value)
                self.assertEqual(self._signed("--archive"), exit_code, self.stderr.getvalue())
                self.assertTrue(self.stderr.getvalue().startswith(f"PipelineFailure: {failure}"), self.stderr.getvalue())
                self.assertNotIn("PipelineQualificationOK", self.stdout.getvalue())
                value = json.loads(self.report_path.read_text())
                self.assertEqual((value["status"], value["failure"]), ("fail", failure))
                self.assertEqual((value["attested"], value["attestation_count"]), (False, 0))
                self.assertFalse(self.catalog_path.exists(), "a failed qualify archives nothing")

    def test_a_result_file_that_is_not_a_required_check_is_never_signed(self):
        """`require_current_pass_results` checks only the required ids, so a
        result file of another id is not accepted. The signer is told the
        accepted ids and refuses the extra file; if a signer signed it anyway,
        the tool refuses the attestation set. Either way the run fails with a
        label and leaves no attestation."""

        def add_an_extra_result(env):
            _emit_check(env, "pipeline_extra_check", digests=False)

        for label, honors, failure, exit_code in (
            ("the signer refuses it", True, "step_failed:check_attestations", 101),
            ("a signer that signs it anyway", False, "check_attestation_count_mismatch", 1),
        ):
            with self.subTest(label):
                self._fresh_run()
                self.signer_honors_ids = honors
                self.after_last_check = add_an_extra_result
                self.assertEqual(self._signed("--archive"), exit_code, self.stderr.getvalue())
                self.assertTrue(self.stderr.getvalue().startswith(f"PipelineFailure: {failure}"), self.stderr.getvalue())
                self.assertEqual(len(self._attestation_calls()), 1)
                self._assert_no_attestation_anywhere()
                self._assert_failed_unattested(failure)
                self.assertFalse(self.catalog_path.exists())
        self.signer_honors_ids = True
        self.after_last_check = None

        with self.subTest("without a signing key the extra result file changes nothing"):
            self._fresh_run()
            self.after_last_check = add_an_extra_result
            self.assertEqual(self._qualify(), 0, self.stderr.getvalue())
            self.assertIn("pipeline_extra_check", {item["check_id"] for item in json.loads(self.report_path.read_text())["checks"]})
            self.after_last_check = None

    def test_the_attestation_signature_has_exactly_its_own_fields(self):
        """The signature of an attestation is its own type: it names the hash
        it signed `attestation_hash` (not `package_hash`) and holds no other
        field."""
        good = {"algorithm": "Ed25519", "key_id": "ci_key", "attestation_hash": _fake_hash("a"), "signature_base64url": "A" * 86}
        cases = (
            (
                "the package field name",
                {**{k: v for k, v in good.items() if k != "attestation_hash"}, "package_hash": _fake_hash("a")},
            ),
            ("an unknown field", {**good, "note": "x"}),
            ("a missing field", {k: v for k, v in good.items() if k != "key_id"}),
            ("another algorithm", {**good, "algorithm": "HS256"}),
            ("a hash that is not a hash", {**good, "attestation_hash": "not-a-hash"}),
            ("a signature of the wrong length", {**good, "signature_base64url": "A" * 85}),
        )
        for label, signature in cases:
            with self.subTest(label):
                self._fresh_run()
                self.attestation_edits = {"pipeline_crash_matrix": {"signature": signature}}
                self.assertEqual(self._signed(), 1)
                self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: check_attestation_invalid")
                self._assert_no_attestation_anywhere()
        self.attestation_edits = {}

    def test_the_signing_key_path_never_reaches_output(self):
        """The key path goes to the writer's environment and nowhere else:
        not the terminal, not either copy of the report, not a failure line."""

        def reached(text):
            return self.KEY_NAME in text or str(self.key.parent) in text

        outputs = []

        def collect():
            outputs.append(self.stdout.getvalue() + self.stderr.getvalue())
            for path in (self.report_path, self.run.run_dir / "qualification-report.json"):
                if path.exists():
                    outputs.append(path.read_text())

        self.assertEqual(self._signed("--archive"), 0, self.stderr.getvalue())
        collect()
        self.assertTrue(self.report_path.is_file())
        self.assertTrue(self.catalog_path.is_file())
        outputs.append(self.catalog_path.read_text())
        for archived in self.catalog_path.parent.glob("lab-records/*.json"):
            outputs.append(archived.read_text())

        for label, knobs in {
            "a missing result": {"silent": {"pipeline_payout_recovery"}},
            "a step that fails": {"attestation_fails": True},
            "a file for another key": {"attestation_key_id": "other_key"},
        }.items():
            self._fresh_run()
            self.silent, self.attestation_fails, self.attestation_key_id = set(), False, None
            for name, value in knobs.items():
                setattr(self, name, value)
            self.assertNotEqual(self._signed(), 0, label)
            collect()
        self._fresh_run()
        self.silent, self.attestation_fails, self.attestation_key_id = set(), False, None
        # A key file that is not there is refused by label, not by path.
        missing = self.tmp / "tcsecretmissing-key.pk8"
        with mock.patch.object(environment, "_invoke", self._invoke):
            self.assertEqual(
                self._main(
                    ["qualify", "--postgres-admin-url", _ADMIN_URL, "--signing-key", str(missing),
                     "--signing-key-id", "ci_key"],
                    cargo=self._fake_cargo(),
                ),
                1,
            )
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: signing_key_unreadable")
        collect()
        self.assertIn("PipelineQualificationOK", outputs[0], "the first output is the signed run's")
        for output in outputs:
            self.assertFalse(reached(output), "the key path is in an output")
            self.assertNotIn("tcsecretmissing", output)

    def test_qualify_refuses_incomplete_or_out_of_range_signing_flags(self):
        cases = (
            (["--signing-key", str(self.key)], "signing_key_incomplete"),
            (["--signing-key-id", "ci_key"], "signing_key_incomplete"),
            (["--signing-key", str(self.key), "--signing-key-id", "bad key id"], "signing_key_id_invalid"),
            (["--signing-key", str(self.key), "--signing-key-id", "ci_key", "--evidence-max-age-seconds", "604801"],
             "evidence_max_age_above_ceiling"),
            (["--evidence-max-age-seconds", "604801"], "evidence_max_age_above_ceiling"),
            (["--signing-key", str(self.key), "--signing-key-id", "ci_key", "--evidence-max-age-seconds", "0"],
             "evidence_max_age_invalid"),
            (["--signing-key", str(self.tmp / "absent.pk8"), "--signing-key-id", "ci_key"], "signing_key_unreadable"),
            (["--signing-key", str(self.tmp), "--signing-key-id", "ci_key"], "signing_key_unreadable"),
        )
        for extra, label in cases:
            with self.subTest(label=label, extra=[part for part in extra if part != str(self.key)]):
                self._fresh_run()
                self.assertEqual(self._qualify(*extra), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {label}")
                self.assertEqual(self._cargo_calls(), [], "refused before any step ran")
        # The ceiling itself is allowed.
        self._fresh_run()
        self.assertEqual(
            self._signed("--evidence-max-age-seconds", "604800"), 0, self.stderr.getvalue()
        )
        [attestation_call] = self._attestation_calls()
        self.assertEqual(attestation_call[4]["TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS"], "604800")

    def test_the_age_flags_match_the_rust_constants(self):
        source = _PROMOTION_SOURCE.read_text()

        def seconds(name):
            match = re.search(rf"pub const {name}: u64 = ([0-9 *]+);", source)
            self.assertIsNotNone(match, name)
            product = 1
            for factor in match.group(1).split("*"):
                product *= int(factor)
            return product

        self.assertEqual(pipeline.DEFAULT_EVIDENCE_AGE_SECONDS, seconds("QUALIFICATION_EVIDENCE_DEFAULT_MAX_AGE_SECONDS"))
        self.assertEqual(pipeline.EVIDENCE_AGE_CEILING_SECONDS, seconds("QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS"))
        self.assertEqual(
            pipeline.parse_args(["qualify"]).evidence_max_age_seconds, pipeline.DEFAULT_EVIDENCE_AGE_SECONDS
        )

    def test_the_corpus_evidence_carries_the_two_digests_the_attestation_reads(self):
        self.assertEqual(self._signed(), 0, self.stderr.getvalue())
        for check_id in checks.REQUIRED_CORPUS_CHECK_IDS:
            evidence = json.loads((self.run.results_dir / f"{check_id}.evidence.json").read_text())
            self.assertRegex(evidence["corpus_digest"], r"^sha256:[0-9a-f]{64}$")
            self.assertRegex(evidence["input_digest"], r"^sha256:[0-9a-f]{64}$")
        hf = json.loads((self.run.results_dir / "pipeline_http_corpus_hf_local.evidence.json").read_text())
        minimal = json.loads((self.run.results_dir / "pipeline_http_corpus_minimal.evidence.json").read_text())
        self.assertNotEqual(hf["corpus_digest"], minimal["corpus_digest"])
        self.assertNotEqual(hf["input_digest"], minimal["input_digest"])


class CorpusEvidenceDigestTests(_CorpusRunCase):
    """The corpus harness's evidence names the corpus it loaded and the
    requests it sent (`corpus_digest`, `input_digest`); `run` and `qualify`
    recompute both from the report and refuse any other value."""

    def _run(self, **harness):
        corpus_path = self.tmp / "corpus.json"
        corpus_path.write_text(json.dumps(_direct_corpus(["alpha_fixture", "beta_fixture"])))
        shutil.rmtree(self.run.run_dir)
        self.run = _scratch_run("corpus")
        self.stdout, self.stderr = io.StringIO(), io.StringIO()
        argv = ["run", "--bundle", "minimal", "--corpus", str(corpus_path), "--postgres-admin-url", _ADMIN_URL]
        return self._main(argv, cargo=self._fake_cargo(**harness)), corpus_path

    def test_the_evidence_digests_are_the_corpus_bytes_and_the_request_hashes(self):
        code, corpus_path = self._run()
        self.assertEqual(code, 0, self.stderr.getvalue())
        evidence = json.loads((self.run.results_dir / "pipeline_http_corpus_minimal.evidence.json").read_text())
        self.assertEqual(evidence["corpus_digest"], _digest(corpus_path.read_bytes()), "one partition: its own digest")
        self.assertEqual(
            evidence["input_digest"],
            _digest(
                results.canonical([_fake_hash("request:alpha_fixture"), _fake_hash("request:beta_fixture")])
            ),
        )

    def test_run_refuses_evidence_whose_digests_differ_from_its_report(self):
        for label, overrides in (
            ("another corpus digest", {"corpus_digest": _fake_hash("another-corpus")}),
            ("another input digest", {"input_digest": _fake_hash("another-input")}),
            ("no corpus digest", {"corpus_digest": _DROP}),
            ("no input digest", {"input_digest": _DROP}),
            ("an extra field", {"another_digest": _fake_hash("x")}),
        ):
            with self.subTest(label):
                code, _ = self._run(evidence_overrides=overrides)
                self.assertEqual(code, 1)
                self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: corpus_evidence_mismatch")

    def test_the_digest_formulas_match_the_rust_unit_test(self):
        """`corpus_evidence_digests_name_the_corpus_and_the_requests` in
        `pipeline_corpus_pg_tests.rs` pins the same three literals."""

        def report(corpus_digests, requests):
            return {
                "partitions": [
                    {"corpus_digest": digest, "fixtures": [{"request_content_hash": item} for item in fixtures]}
                    for digest, fixtures in zip(corpus_digests, requests)
                ]
            }

        one, two = "sha256:" + "1" * 64, "sha256:" + "2" * 64
        a, b, c = ("sha256:" + letter * 64 for letter in "abc")
        self.assertEqual(corpus.evidence_digests(report([one], [[a, b]])), (one, "sha256:b7cc444b1f09d1e380d2a82f6ae10447932ce3b91f31b0ab2e659f7a7fdea15f"))
        self.assertEqual(
            corpus.evidence_digests(report([one, two], [[a, b], [c]])),
            (
                "sha256:636d591478be5cb76f2025a46b17a229fa29ef91724891793005baa9eef2100e",
                "sha256:208dcba10edb6eea7324c3be4765e976cb578c1d9715d91e60d4042df6650461",
            ),
        )


class KeygenTests(_CorpusRunCase):
    def setUp(self):
        super().setUp()
        self.out = self.tmp / "tcsecretdir-91b2" / "tcsecretkey-1c0f.pk8"
        self.trusted = self.tmp / "tcsecretdir-91b2" / "trusted-check-key.json"
        self.mode = 0o600
        self.key_id_in_file = None

    def _fake_cargo(self, **overrides):
        def fake_cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            key = Path(env["TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT"])
            key.write_bytes(b"fake pkcs8 bytes")
            key.chmod(self.mode)
            Path(env["TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT"]).write_text(
                json.dumps(
                    {
                        "key_id": self.key_id_in_file or env["TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_KEY_ID"],
                        "public_key_base64url": "A" * 43,
                    }
                )
            )

        return fake_cargo_test

    def _keygen(self, *extra, output=None, trusted=None, key_id="local_check_key"):
        argv = ["keygen", "--output", str(output or self.out), "--key-id", key_id,
                "--trusted-key-output", str(trusted or self.trusted), *extra]
        return self._main(argv, cargo=self._fake_cargo())

    def test_keygen_runs_the_key_writer_and_prints_no_path(self):
        self.assertEqual(self._keygen(), 0, self.stderr.getvalue())
        [(_, step, cargo_args, test_filter, env, exact, ignored)] = self._cargo_calls()
        self.assertRegex(step, r"^[a-z0-9_]{1,64}$")
        self.assertEqual((cargo_args, test_filter, exact, ignored), (_INGEST_ARGS, _KEY_WRITER, True, True))
        self.assertEqual(
            {key: value for key, value in env.items() if key.startswith("TRACE_COMMONS_")},
            {
                "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT": str(self.out.resolve()),
                "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_KEY_ID": "local_check_key",
                "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT": str(self.trusted.resolve()),
            },
        )
        self.assertEqual(self.stdout.getvalue().strip(), "PipelineKeygenOK: key_id=local_check_key")
        self.assertEqual(self.stderr.getvalue(), "")
        for output in (self.stdout.getvalue(), self.stderr.getvalue()):
            self.assertNotIn("tcsecret", output)
            self.assertNotIn(str(self.tmp), output)
        self.assertEqual(self.out.stat().st_mode & 0o777, 0o600)

    def test_keygen_refuses_what_it_could_overwrite_or_leave_readable(self):
        self.out.parent.mkdir(parents=True)
        for label, setup, expected in (
            ("an existing key file", lambda: self.out.write_bytes(b"older key"), "signing_key_output_exists"),
            ("an existing trusted key file", lambda: self.trusted.write_text("{}"), "signing_key_output_exists"),
        ):
            with self.subTest(label):
                for path in (self.out, self.trusted):
                    path.unlink(missing_ok=True)
                self.calls.clear()
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                setup()
                existing = {path: path.read_bytes() for path in (self.out, self.trusted) if path.exists()}
                self.assertEqual(self._keygen(), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {expected}")
                self.assertEqual(self._cargo_calls(), [], "refused before the key writer starts")
                self.assertEqual({path: path.read_bytes() for path in existing}, existing, "nothing was changed")
                self.assertNotIn("tcsecret", self.stderr.getvalue())
        for path in (self.out, self.trusted):
            path.unlink(missing_ok=True)

        for label, kwargs, expected in (
            ("one path for both files", {"output": self.out, "trusted": self.out}, "keygen_outputs_must_differ"),
            ("a key id that is not an identifier", {"key_id": "bad key id"}, "signing_key_id_invalid"),
        ):
            with self.subTest(label):
                self.calls.clear()
                self.stdout, self.stderr = io.StringIO(), io.StringIO()
                self.assertEqual(self._keygen(**kwargs), 1)
                self.assertEqual(self.stderr.getvalue().strip(), f"PipelineFailure: {expected}")
                self.assertEqual(self._cargo_calls(), [])

        with self.subTest("a key file that others can read"):
            self.calls.clear()
            self.stdout, self.stderr = io.StringIO(), io.StringIO()
            self.mode = 0o644
            self.assertEqual(self._keygen(), 1)
            self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: signing_key_mode_invalid")
            self.assertNotIn("PipelineKeygenOK", self.stdout.getvalue())
            self.mode = 0o600

        with self.subTest("a trusted key that names another key id"):
            for path in (self.out, self.trusted):
                path.unlink(missing_ok=True)
            self.stdout, self.stderr = io.StringIO(), io.StringIO()
            self.key_id_in_file = "another_key"
            self.assertEqual(self._keygen(), 1)
            self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: trusted_key_invalid")
            self.assertNotIn("tcsecret", self.stderr.getvalue() + self.stdout.getvalue())


class RevisionTests(_CorpusRunCase):
    def test_revision_prints_the_tree_hash(self):
        self.assertEqual(self._main(["revision"]), 0, self.stderr.getvalue())
        self.assertEqual(self.stdout.getvalue(), self.run.code_revision_hash + "\n")
        self.assertEqual(self.stderr.getvalue(), "")
        self.assertEqual(self.calls, [], "no child process")
        self.assertRegex(self.stdout.getvalue(), r"^sha256:[0-9a-f]{64}\n\Z")


class EphemeralCommandTests(unittest.TestCase):
    """`revision` and `keygen` keep nothing, so they leave no run directory
    (the real `Run`, in a scratch root): not when they succeed, and not when
    they are refused before they ran anything. A failed step keeps its log in
    the run directory and the failure line names it."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.stdout, self.stderr = io.StringIO(), io.StringIO()
        self.out = self.root / "keys" / "check-key.pk8"
        self.trusted = self.root / "keys" / "check-key.json"

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def _runs(self):
        directory = self.root / ".local" / "pipeline" / "runs"
        return sorted(path.name for path in directory.iterdir()) if directory.exists() else []

    def _main(self, argv, cargo=None):
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(environment, "ROOT", self.root))
            stack.enter_context(mock.patch.object(pipeline, "ROOT", self.root))
            stack.enter_context(mock.patch.object(environment, "_code_revision_hash", lambda: _fake_hash("tree")))
            if cargo is not None:
                stack.enter_context(mock.patch.object(pipeline, "cargo_test", cargo))
            stack.enter_context(contextlib.redirect_stdout(self.stdout))
            stack.enter_context(contextlib.redirect_stderr(self.stderr))
            return pipeline.main(argv)

    def _keygen_argv(self):
        return ["keygen", "--output", str(self.out), "--key-id", "local_check_key", "--trusted-key-output", str(self.trusted)]

    def _writing_cargo(self, run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
        key = Path(env["TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT"])
        key.write_bytes(b"fake pkcs8 bytes")
        key.chmod(0o600)
        Path(env["TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT"]).write_text(
            json.dumps({"key_id": "local_check_key", "public_key_base64url": "A" * 43})
        )
        # What a real step leaves: its log, in the run directory.
        run.log_path(step).write_text("cargo output")

    def test_revision_leaves_no_run_directory(self):
        self.assertEqual(self._main(["revision"]), 0, self.stderr.getvalue())
        self.assertEqual(self.stdout.getvalue(), _fake_hash("tree") + "\n")
        self.assertEqual(self._runs(), [])

    def test_a_keygen_that_succeeds_leaves_no_run_directory(self):
        self.assertEqual(self._main(self._keygen_argv(), cargo=self._writing_cargo), 0, self.stderr.getvalue())
        self.assertEqual(self.stdout.getvalue().strip(), "PipelineKeygenOK: key_id=local_check_key")
        self.assertEqual(self._runs(), [])
        self.assertTrue(self.out.is_file() and self.trusted.is_file(), "the keys it wrote are kept")

    def test_a_keygen_that_is_refused_before_it_ran_leaves_no_run_directory(self):
        self.out.parent.mkdir(parents=True)
        self.out.write_bytes(b"an older key")
        self.assertEqual(self._main(self._keygen_argv(), cargo=self._writing_cargo), 1)
        self.assertEqual(self.stderr.getvalue().strip(), "PipelineFailure: signing_key_output_exists")
        self.assertEqual(self._runs(), [])
        self.assertEqual(self.out.read_bytes(), b"an older key")

    def test_a_keygen_whose_step_failed_keeps_the_log_the_failure_names(self):
        def failing(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            log = run.log_path(step)
            log.write_text("cargo output")
            raise errors.StepFailed(step, 101, log)

        self.assertEqual(self._main(self._keygen_argv(), cargo=failing), 101)
        [name] = self._runs()
        self.assertIn(f".local/pipeline/runs/{name}/logs/keygen.log", self.stderr.getvalue())
        self.assertTrue((self.root / ".local" / "pipeline" / "runs" / name / "logs" / "keygen.log").is_file())

    def test_other_commands_keep_their_run_directory(self):
        corpus_path = self.root / "corpus.json"
        corpus_path.write_text("{}")
        self.assertEqual(self._main(["run", "--bundle", "minimal", "--corpus", str(corpus_path)]), 1)
        self.assertEqual(len(self._runs()), 1, "only revision and keygen remove theirs")


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



class CodeRevisionExcludeTests(unittest.TestCase):
    """Final fix wave (G22): the code revision hashes the tracked files and
    the untracked files that the repository's own `.gitignore` files do not
    ignore. A host's `.git/info/exclude` and a user's global excludes file
    hide nothing, so one checkout gives one revision on every host. An
    untracked `.cargo` directory is the one thing it leaves out by itself.
    These tests run the real `git` in a temporary repository."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="revision-excludes-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        self.repo.mkdir()
        global_ignore = self.tmp / "global-ignore"
        global_ignore.write_text("globally_ignored.txt\n")
        global_config = self.tmp / "gitconfig"
        global_config.write_text(f"[core]\n\texcludesFile = {global_ignore}\n")
        env = mock.patch.dict(
            os.environ,
            {"GIT_CONFIG_GLOBAL": str(global_config), "GIT_CONFIG_NOSYSTEM": "1"},
        )
        env.start()
        self.addCleanup(env.stop)
        self._git("init", "-q")
        (self.repo / ".gitignore").write_text("ignored.txt\n")
        (self.repo / "tracked.txt").write_text("tracked\n")
        self._git("add", ".gitignore", "tracked.txt")
        info = self.repo / ".git" / "info"
        info.mkdir(exist_ok=True)
        (info / "exclude").write_text("locally_excluded.txt\n")
        root = mock.patch.object(environment, "ROOT", self.repo)
        root.start()
        self.addCleanup(root.stop)

    def _git(self, *args):
        subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True)

    def _revision_after_writing(self, name, content):
        (self.repo / name).write_text(content)
        return environment._code_revision_hash()

    def test_local_and_global_excludes_hide_no_file(self):
        for name in ("locally_excluded.txt", "globally_ignored.txt"):
            first = self._revision_after_writing(name, "one\n")
            second = self._revision_after_writing(name, "two\n")
            self.assertNotEqual(first, second, name)

    def test_the_repository_gitignore_still_applies(self):
        first = self._revision_after_writing("ignored.txt", "one\n")
        second = self._revision_after_writing("ignored.txt", "two\n")
        self.assertEqual(first, second)
        third = self._revision_after_writing("untracked.txt", "new\n")
        self.assertNotEqual(second, third)

    def test_an_untracked_cargo_config_is_not_part_of_the_revision(self):
        """Review round 1 of #1240, point 11: the repository's `.gitignore`
        has no `.cargo/` line (it would keep a new cargo config, not yet
        added, out of `git status`), and
        the revision leaves out an untracked `.cargo/` directory itself, at
        the root and nested, as that line did."""
        base = environment._code_revision_hash()
        (self.repo / "crates" / "one").mkdir(parents=True)
        (self.repo / "crates" / "one" / "lib.rs").write_text("// one\n")
        with_crate = environment._code_revision_hash()
        self.assertNotEqual(base, with_crate)
        for directory in (self.repo / ".cargo", self.repo / "crates" / "one" / ".cargo"):
            directory.mkdir()
            for content in ("[build]\njobs = 5\n", "[build]\njobs = 6\n"):
                (directory / "config.toml").write_text(content)
                self.assertEqual(environment._code_revision_hash(), with_crate, directory)
        # Only a directory of that name: a file named like it elsewhere, and
        # a file whose name only starts with it, are part of the tree.
        first = self._revision_after_writing(".cargo-notes.txt", "one\n")
        second = self._revision_after_writing(".cargo-notes.txt", "two\n")
        self.assertNotEqual(first, second)

    def test_a_tracked_cargo_config_is_part_of_the_revision(self):
        """A checked-in `.cargo/config.toml` is visible to git (no ignore
        line hides it) and changes the revision with each edit, at the root
        and nested."""
        (self.repo / "crates" / "one").mkdir(parents=True)
        for directory in (self.repo / ".cargo", self.repo / "crates" / "one" / ".cargo"):
            directory.mkdir()
            config = directory / "config.toml"
            config.write_text("[build]\njobs = 5\n")
            untracked = environment._code_revision_hash()
            status = subprocess.run(
                ["git", "status", "--porcelain", "--untracked-files=all"],
                cwd=self.repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            self.assertIn(str(config.relative_to(self.repo)), status, "git shows the file")
            self._git("add", str(config.relative_to(self.repo)))
            tracked = environment._code_revision_hash()
            self.assertNotEqual(untracked, tracked, directory)
            config.write_text("[build]\njobs = 6\n")
            self.assertNotEqual(environment._code_revision_hash(), tracked, directory)

    def test_the_repository_gitignore_has_no_cargo_line(self):
        """The real `.gitignore`: no line hides `.cargo`, so a checked-in
        cargo config shows in `git status`."""
        lines = (Path(__file__).resolve().parents[2] / ".gitignore").read_text().splitlines()
        self.assertEqual([line for line in lines if line.strip().strip("/") == ".cargo"], [])
class ActivityMissionInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).with_name("pipeline-deployment-inventory.py")
        spec = importlib.util.spec_from_file_location("deployment_inventory", path)
        cls.inventory = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.inventory)

    def test_catalogue_and_account_status_keep_their_distinct_contract_families(self):
        self.assertEqual(self.inventory.classify_route("/v1/activity-missions"), ("EXP-004", "CRD-004"))
        self.assertEqual(self.inventory.classify_route("/v1/account/activity-missions/status"), ("AUTH-001", "AUTH-004"))
        for path in ("/v1/activity-missions-extra", "/v1/activity-missions/status", "/v1/activity-missions/"):
            self.assertIsNone(self.inventory.classify_route(path), path)

    def test_the_credit_estimate_table_is_a_public_catalogue(self):
        """The local credit estimate table is public, non-personal data a
        client fetches before it has an account, like the mission catalogue."""
        self.assertEqual(self.inventory.classify_route("/v1/credit-estimate/table"), ("EXP-004", "CRD-004"))
        for path in ("/v1/credit-estimate", "/v1/credit-estimate/table/extra", "/v1/credit-estimate/tables"):
            self.assertIsNone(self.inventory.classify_route(path), path)

    def test_new_catalogue_is_in_inventory_and_unknown_interfaces_still_fail_closed(self):
        inventory = self.inventory.build_inventory()
        catalogue = [row for row in inventory["routes"] if row["path"] == "/v1/activity-missions"]
        self.assertEqual(len(catalogue), 1)
        self.assertEqual(catalogue[0]["contracts"], ["EXP-004", "CRD-004"])
        with mock.patch.object(self.inventory, "source_routes", return_value=["/v1/activity-missions-future"]):
            with self.assertRaisesRegex(ValueError, "unclassified deployment interface"):
                self.inventory.build_inventory()



# ---------------------------------------------------------------------------
# Spec 2026-10-08 Slice B-1: `pipeline.py promote` and `hf-pin record`, the
# operator-run production checks. Every network and bucket call is an
# injected callable here; nothing below downloads or touches a store.
# ---------------------------------------------------------------------------

from pipeline_tooling import promote  # noqa: E402

_PROMOTE_FEATURES = ("--features", "near-ai-scorer,gcs-client,gcp-kms")
_HEX40 = "0123456789abcdef0123456789abcdef01234567"


def _fake_policy(name, phase, data_artifact_hashes=()):
    return {
        "policy_id": f"trace_commons.{phase}.{name}",
        "implementation_id": f"trace_commons.{phase}.{name}.v1",
        "configuration_hash": _digest(f"{name}-{phase}".encode()),
        "data_artifact_hashes": list(data_artifact_hashes),
        "projection_ids": [],
    }


def _rederive_bundle_id(package):
    """Re-derives the bundle identifier after a test edits the manifest, so
    the package stays valid and only its hash moves."""
    package["bundle_id"] = _digest(promote._manifest_canonical_bytes(package["manifest"]))
    return package


def _fake_signed_package(name="production"):
    """A signed package the way `pipeline.py package` writes one: a valid
    `BundlePackage` (artifact bytes that hash to their keys, a bundle
    identifier derived from the manifest) and a signature whose
    `package_hash` is `promote.bundle_package_hash` of it. That function is
    pinned to the Rust rule by `test_package_digests_match_the_rust_vector`,
    whose vector a Rust test holds to `BundlePackage::package_hash`, so using
    it here is not an oracle of the tool's own making. The signature bytes
    are not verified offline."""
    data = {f"{name}-{phase}".encode() for phase in ("admission", "review", "score", "settle")}
    data |= {f"{name}-embedder".encode(), f"{name}-scorer".encode()}
    score_data = [_digest(f"{name}-scorer".encode()), _digest(f"{name}-embedder".encode())]
    package = _rederive_bundle_id({
        "bundle_id": None,
        "manifest": {
            "format_version": 2,
            "admission": _fake_policy(name, "admission"),
            "review": _fake_policy(name, "review"),
            "score": _fake_policy(name, "score", score_data),
            "settle": _fake_policy(name, "settle"),
            "instruments": {
                "storage_rebate": {"kind": "credit_account", "network": "pipeline-test", "contract": "storage-rebate", "decimals": 0},
            },
        },
        "artifacts": {_digest(item): item.hex() for item in data},
    })
    return {
        "package": package,
        "signature": {
            "algorithm": "Ed25519",
            "key_id": "production_package_key",
            "package_hash": promote.bundle_package_hash(package),
            "signature_base64url": "A" * 86,
        },
    }


def _package_triple(signed):
    manifest = signed["package"]["manifest"]
    return (
        signed["signature"]["package_hash"],
        _digest(results.canonical({phase: manifest[phase]["configuration_hash"] for phase in ("admission", "review", "score", "settle")})),
        _digest(results.canonical(sorted(manifest["score"]["data_artifact_hashes"]))),
    )


def _hf_network_manifest(**overrides):
    manifest = {
        "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
        "source": {
            "repository": "jedisct1/security-audits",
            "revision": _HEX40,
            "split": "train",
            "translator": "swival",
            "bootstrap_count": 1,
            "holdout_count": 1,
            "min_words": 1,
            "max_words": 2000,
            "expected_instrument_count": 1,
        },
        "source_digest": _fake_hash("source"),
        "order_digest": _fake_hash("order"),
        "bootstrap_corpus_digest": _fake_hash("bootstrap"),
        "holdout_corpus_digest": _fake_hash("holdout"),
        "configuration_digest": _fake_hash("configuration"),
        "sample_count": 2,
        "bootstrap_count": 1,
        "holdout_count": 1,
        "contains_raw_trace_text": False,
        "contains_contributor_identity": False,
    }
    manifest.update(overrides)
    return manifest


# The seed's credit counts under a zero `NoveltyUtility` delta (the pilot's).
_UNCREDITED_SEED = {
    "credit_delta_zero": True,
    "adapter_request_count": 0,
    "completed_settlement_count": 0,
    "completed_credit_event_count": 0,
}


def _remote_report(**overrides):
    report = {
        "schema": "trace_commons.pipeline_remote_restore_report.v1",
        "object_store_kind": "gcs",
        "object_count": 42,
        "artifact_fingerprint": _fake_hash("artifacts"),
        "restored_artifact_fingerprint": _fake_hash("artifacts"),
        "versioning_enabled": True,
        "kek_unwrap_verified_count": 42,
        "seed_database_fingerprint": _fake_hash("database"),
        "resumed_database_fingerprint": _fake_hash("database"),
        "pending_runs_resumed": 1,
        "duplicate_effects": 0,
    }
    report.update(overrides)
    return report


class _PromoteCase(unittest.TestCase):
    """A real `Run` and the real `main`, rooted in a scratch directory: the
    tree hash, cargo, the HF export, and the remote restore harness are the
    only things replaced."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.stdout, self.stderr = io.StringIO(), io.StringIO()
        self.tree = _fake_hash("tree")
        self.calls = []
        self.signed = _fake_signed_package()
        self.package_path = self.root / "inputs" / "production-package.json"
        self.package_path.parent.mkdir(parents=True)
        self.package_path.write_text(json.dumps(self.signed))
        self.key_path = self.root / "inputs" / "production-package-key.json"
        self.key_path.write_text(json.dumps({"key_id": "production_package_key", "public_key_base64url": "A" * 43}))
        self.manifest = _hf_network_manifest()
        self.downloaded = 2
        self.report = _remote_report()
        self.harness_calls = []
        self.harness_admin_urls = []
        # The deployment's env file: variables `package-checks` reads, and
        # three it must never pass on (the live index root among them).
        self.env_path = self.root / "inputs" / "ingest.env"
        self.env_path.write_text(
            "# the deployment's env file\n"
            "TRACE_COMMONS_NEAR_AI_BASE_URL=https://near-ai.invalid/v1\n"
            'TRACE_COMMONS_NEAR_AI_API_KEY="near-ai-test-key"\n'
            "TRACE_COMMONS_NEAR_AI_MODEL=Qwen/Qwen3.6-35B-A3B-FP8\n"
            "TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS=90\n"
            "TRACE_COMMONS_VECTOR_INDEX_DIM=1024\n"
            "TRACE_COMMONS_VECTOR_INDEX_ROOT=/var/lib/trace-commons-vector-index\n"
            "TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/trace-commons-embedder\n"
            "TRACE_COMMONS_DATABASE_URL=postgres://ingest@127.0.0.1/trace_commons\n"
            "TRACE_COMMONS_ARTIFACT_MASTER_KEY_HEX=00112233\n"
        )

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def _runs_dir(self):
        return self.root / ".local" / "pipeline" / "runs"

    def _runs(self):
        directory = self._runs_dir()
        return sorted(path.name for path in directory.iterdir()) if directory.exists() else []

    def _fake_cargo(self, run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
        self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
        if test_filter == _ATTESTATION_WRITER:
            staging = Path(env["TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR"])
            results_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
            for check_id in env["TRACE_COMMONS_PIPELINE_CHECK_IDS"].split(","):
                raw = json.loads((results_dir / f"{check_id}.result.json").read_text())
                (staging / f"{check_id}.attestation.json").write_text(
                    json.dumps(_attestation_for(raw, env["TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID"],
                                                int(env["TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS"])))
                )

    def _fake_export(self, run, step, fields, output_dir, cache_dir):
        self.calls.append(("export", step, dict(fields), Path(output_dir), Path(cache_dir)))
        output_dir = Path(output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "source-manifest.json").write_text(json.dumps(self.manifest))
        snapshot = Path(cache_dir) / "datasets--jedisct1--security-audits" / "snapshots" / "abc"
        snapshot.mkdir(parents=True, exist_ok=True)
        for index in range(self.downloaded):
            (snapshot / f"{index:02}.jsonl").write_text("{}\n")

    def _fake_harness(self, run, step, source_store, scratch_store, report_path, *, postgres_admin_url=None):
        self.harness_calls.append((step, source_store, scratch_store, Path(report_path)))
        self.harness_admin_urls.append((step, source_store, scratch_store, Path(report_path), postgres_admin_url))
        Path(report_path).write_text(json.dumps(self.report))

    def _main(self, argv, *, ci=False, harness=True, tree=None):
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(environment, "ROOT", self.root))
            stack.enter_context(mock.patch.object(pipeline, "ROOT", self.root))
            stack.enter_context(mock.patch.object(environment, "_code_revision_hash", lambda: tree or self.tree))
            stack.enter_context(mock.patch.object(pipeline, "cargo_test", self._fake_cargo))
            stack.enter_context(mock.patch.object(promote, "run_hf_export", self._fake_export))
            if harness:
                stack.enter_context(mock.patch.object(promote, "run_remote_restore_harness", self._fake_harness))
            environ = stack.enter_context(mock.patch.dict(os.environ))
            environ.pop("CI", None)
            if ci:
                environ["CI"] = "true"
            stack.enter_context(contextlib.redirect_stdout(self.stdout))
            stack.enter_context(contextlib.redirect_stderr(self.stderr))
            return pipeline.main(argv)

    def _init(self):
        before = set(self._runs())
        code = self._main(["promote", "init", "--package", str(self.package_path), "--trusted-key", str(self.key_path)])
        self.assertEqual(code, 0, self.stderr.getvalue())
        [run_id] = sorted(set(self._runs()) - before)
        self.stdout.seek(0)
        self.stdout.truncate()
        return run_id

    def _results_dir(self, run_id):
        return self._runs_dir() / run_id / "results"

    def _result(self, run_id, check_id):
        directory = self._results_dir(run_id)
        return (
            json.loads((directory / f"{check_id}.result.json").read_text()),
            json.loads((directory / f"{check_id}.evidence.json").read_text()),
        )

    def _failure(self):
        return self.stderr.getvalue().strip()

    def _write_production_result(self, production_run_id, check_id, *, package=None, evidence=None, **overrides):
        """A passing result of the production run. A package check's evidence
        says `harness_assembly: production`, as the harness writes it in that
        mode, unless `evidence` replaces it."""
        run_id = production_run_id
        package_hash, configuration_digest, dependency_digest = package or _package_triple(self.signed)
        record = json.loads((self._runs_dir() / run_id / promote.PROMOTE_RUN_FILE).read_text())
        if evidence is None:
            evidence = {"observed": check_id}
            if check_id in promote.package_check_ids():
                evidence["harness_assembly"] = "production"
        raw = {
            "schema": results.SCHEMA,
            "run_id": run_id,
            "check_id": check_id,
            "status": "pass",
            "code_revision_hash": record["code_revision_hash"],
            "package_hash": package_hash,
            "configuration_digest": configuration_digest,
            "dependency_digest": dependency_digest,
            "observed_at": _iso(datetime.now(timezone.utc)),
            "evidence_hash": _digest(results.canonical(evidence)),
            "safe_blockers": [],
        }
        raw.update(overrides)
        directory = self._results_dir(run_id)
        (directory / f"{check_id}.result.json").write_text(json.dumps(raw))
        (directory / f"{check_id}.evidence.json").write_text(json.dumps(evidence))
        return raw


def _attestation_for(raw, key_id, maximum_age_seconds):
    return {
        "schema": results.ATTESTATION_SCHEMA,
        "result": raw,
        "maximum_age_seconds": maximum_age_seconds,
        "corpus_digest": None,
        "input_digest": None,
        "signature": {
            "algorithm": "Ed25519",
            "key_id": key_id,
            "attestation_hash": _fake_hash(raw["check_id"]),
            "signature_base64url": "A" * 86,
        },
    }


class PromoteListTests(unittest.TestCase):
    def test_promotion_only_ids_and_labels_match_the_rust_source(self):
        self.assertEqual(frozenset(promote.PROMOTION_ONLY_CHECK_IDS), frozenset(_rust_check_list("PROMOTION_ONLY_CHECKS")))
        source = _PROMOTION_SOURCE.read_text()
        for name, value in (
            ("REMOTE_RESTORE_CHECK_ID", promote.REMOTE_RESTORE_CHECK_ID),
            ("RESTORE_DRILL_CHECK_ID", checks.RESTORE_CHECK_ID),
            ("LOCAL_RESTORE_BLOCKER_LABEL", pipeline.RESTORE_SAFE_BLOCKER),
        ):
            self.assertIn(f'pub const {name}: &str = "{value}";', source, name)

    def test_the_production_run_holds_the_package_checks_and_the_promotion_only_three(self):
        package = frozenset(check_id for check_id, spec in checks.required_specs().items() if spec.digests_required)
        self.assertEqual(frozenset(promote.package_check_ids()), package)
        self.assertEqual(
            frozenset(promote.production_check_ids()), package | frozenset(promote.PROMOTION_ONLY_CHECK_IDS)
        )
        self.assertEqual(frozenset(promote.mechanics_check_ids()), checks.REQUIRED_CHECK_IDS - package)
        self.assertEqual(
            frozenset(promote.mechanics_check_ids()) | frozenset(promote.production_check_ids()),
            frozenset(_promotion_required_checks()),
        )
        self.assertEqual(frozenset(promote.mechanics_check_ids()) & frozenset(promote.production_check_ids()), frozenset())

    def test_package_digests_match_the_rust_vector(self):
        """`fixtures/pipeline-package-digests-vector.json` is written by Rust
        and held to `package_digests` in `versioned_pipeline_qualification.rs`
        by `the_cross_language_package_digest_vector_follows_the_rust_rule`.
        The package hash is the hash of `BundlePackage::canonical_bytes`, a
        binary encoding, not of the package's JSON."""
        vector = json.loads((Path(__file__).resolve().parent / "fixtures" / "pipeline-package-digests-vector.json").read_text())
        signed = {"package": vector["package"], "signature": {"package_hash": vector["package_hash"]}}
        self.assertEqual(
            promote.package_digests(signed),
            (vector["package_hash"], vector["configuration_digest"], vector["dependency_digest"]),
        )
        self.assertEqual(promote.bundle_package_hash(vector["package"]), vector["package_hash"])

        # Where Rust's `canonical_bytes` errors, Python refuses; it never
        # produces a hash.
        def edited(edit):
            package = json.loads(json.dumps(vector["package"]))
            edit(package)
            return package

        for edit in (
            lambda p: p["manifest"].update(format_version=1),
            lambda p: p["manifest"]["score"]["projection_ids"].append("pipeline-test-projection-v1"),
            lambda p: p.update(bundle_id=_fake_hash("another bundle")),
            lambda p: p["artifacts"].popitem(),
            lambda p: p["artifacts"].update({k: v + "00" for k, v in list(p["artifacts"].items())[:1]}),
            # A lone surrogate: `json.loads` accepts the escape, and no UTF-8
            # encoding of it exists.
            lambda p: p["manifest"]["score"].update(policy_id="\udc80"),
        ):
            with self.assertRaises(errors.ToolingError) as ctx:
                promote.bundle_package_hash(edited(edit))
            self.assertEqual(str(ctx.exception), "promote_package_invalid")

    def test_package_digests_follow_the_rust_rule(self):
        signed = _fake_signed_package()
        self.assertEqual(promote.package_digests(signed), _package_triple(signed))
        signed["signature"]["package_hash"] = _fake_hash("another package")
        with self.assertRaises(errors.ToolingError) as ctx:
            promote.package_digests(signed)
        self.assertEqual(str(ctx.exception), "promote_package_hash_mismatch")
        with self.assertRaises(errors.ToolingError) as ctx:
            promote.package_digests({"package": {}})
        self.assertEqual(str(ctx.exception), "promote_package_invalid")


class PromoteTests(_PromoteCase):
    def test_promote_refuses_in_ci(self):
        run_id = self._init()
        for argv in (
            ["promote", "init", "--package", str(self.package_path), "--trusted-key", str(self.key_path)],
            ["promote", "package-checks", "--run-id", run_id, "--env-file", str(self.env_path)],
            ["promote", "hf-canary", "--run-id", run_id, "--pin", str(self.root / "pin.json")],
            ["promote", "remote-restore", "--run-id", run_id, "--source-store", "live-bucket", "--scratch-store", "scratch-bucket"],
            ["promote", "adapters", "--run-id", run_id],
            ["promote", "sign", "--run-id", run_id, "--signing-key", str(self.key_path), "--signing-key-id", "k"],
            ["promote", "assemble", "--run-id", run_id, "--mechanics-run-id", "q00000000", "--output", str(self.root / "out")],
            ["hf-pin", "record", "--revision", _HEX40, "--output", str(self.root / "pin.json")],
        ):
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._main(argv, ci=True), 1, argv)
            self.assertEqual(self._failure(), "PipelineFailure: promote_refused_in_ci", argv)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.harness_calls, [])
        self.assertEqual(self._runs(), [run_id], "a refused command leaves no run directory")

    def test_promote_requires_near_ai_build(self):
        self.assertEqual(promote.PROMOTE_CARGO_ARGS, (*_INGEST_ARGS, *_PROMOTE_FEATURES))
        run_id = self._init()
        for check_id in promote.production_check_ids():
            self._write_production_result(run_id, check_id)
        key = self.root / "inputs" / "check-key.pk8"
        key.write_bytes(b"fake pkcs8")
        code = self._main(["promote", "sign", "--run-id", run_id, "--signing-key", str(key), "--signing-key-id", "operator_check_key"])
        self.assertEqual(code, 0, self.stderr.getvalue())
        cargo_calls = [call for call in self.calls if call[0] == "cargo"]
        self.assertTrue(cargo_calls)
        for call in cargo_calls:
            self.assertEqual(call[2], promote.PROMOTE_CARGO_ARGS)

    def test_promote_init_records_the_run_revision_and_package(self):
        run_id = self._init()
        self.assertRegex(run_id, r"^q[0-9a-f]{8}$")
        record = json.loads((self._runs_dir() / run_id / promote.PROMOTE_RUN_FILE).read_text())
        package_hash, configuration_digest, dependency_digest = _package_triple(self.signed)
        self.assertEqual(record["schema"], promote.PROMOTE_RUN_SCHEMA)
        self.assertEqual(record["run_id"], run_id)
        self.assertEqual(record["code_revision_hash"], self.tree)
        self.assertEqual(
            (record["package_hash"], record["configuration_digest"], record["dependency_digest"]),
            (package_hash, configuration_digest, dependency_digest),
        )
        self.assertEqual(json.loads((self._runs_dir() / run_id / promote.PACKAGE_FILE).read_text()), self.signed)

        # A valid package whose hash is not the signed one is refused.
        tampered = _fake_signed_package()
        tampered["package"]["manifest"]["score"]["policy_id"] = "trace_commons.score.edited"
        _rederive_bundle_id(tampered["package"])
        self.package_path.write_text(json.dumps(tampered))
        code = self._main(["promote", "init", "--package", str(self.package_path), "--trusted-key", str(self.key_path)])
        self.assertEqual(code, 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_package_hash_mismatch")

        # A package that does not validate (an edited configuration hash no
        # longer names an artifact, and the bundle identifier no longer
        # derives) is refused before any hash is compared.
        tampered = _fake_signed_package()
        tampered["package"]["manifest"]["score"]["configuration_hash"] = _fake_hash("edited")
        self.package_path.write_text(json.dumps(tampered))
        code = self._main(["promote", "init", "--package", str(self.package_path), "--trusted-key", str(self.key_path)])
        self.assertEqual(code, 1)
        self.assertEqual(self._failure().splitlines()[-1], "PipelineFailure: promote_package_invalid")

    def test_init_prints_the_run_id_and_revision_and_no_path(self):
        code = self._main(["promote", "init", "--package", str(self.package_path), "--trusted-key", str(self.key_path)])
        self.assertEqual(code, 0, self.stderr.getvalue())
        [run_id] = self._runs()
        output = self.stdout.getvalue()
        self.assertIn(f"run_id={run_id}", output)
        self.assertIn(f"code_revision_hash={self.tree}", output)
        self.assertNotIn(str(self.root), output)

    def test_promote_commands_reopen_the_init_run(self):
        run_id = self._init()
        env_file = ["--env-file", str(self.env_path)]
        code = self._main(["promote", "package-checks", "--run-id", run_id, *env_file], tree=_fake_hash("another tree"))
        self.assertEqual(code, 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_code_revision_changed")
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._main(["promote", "package-checks", "--run-id", "q00000000", *env_file]), 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_run_missing")
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._main(["promote", "package-checks", "--run-id", "../escape", *env_file]), 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_run_id_invalid")
        self.assertEqual(self._runs(), [run_id], "only init keeps a run directory")



def _network_pin(manifest):
    """`pin-local.json` as `hf-pin record` turns it into a network pin: no
    local directory, a revision, and the digests `manifest` computed."""
    local = json.loads(promote.HF_LOCAL_PIN.read_text())
    pin = {key: value for key, value in local.items() if key != "local_jsonl_dir"}
    pin["revision"] = _HEX40
    for field in promote.HF_PIN_DIGEST_FIELDS:
        pin[field] = manifest[field]
    return pin


class PromotePackageChecksTests(_PromoteCase):
    """`promote package-checks` (spec B-D1, B-2): the four package checks on
    the production assembly. `run_package_checks`, which starts the
    harnesses, is replaced by a fake that writes the four results the way
    the production harness does; `ProductionHarnessRunnerTests` covers it."""

    def setUp(self):
        super().setUp()
        # The committed pin, where `promote` looks for it under the scratch root.
        self.pin_path = self.root / promote.HF_NETWORK_PIN_PATH
        self.pin_path.parent.mkdir(parents=True, exist_ok=True)
        self.pin_path.write_text(json.dumps(_network_pin(self.manifest)))
        self.hook_calls = []
        self.hook_evidence = {}
        self.hook_package = None

    def _fake_package_checks(self, run, *, harness_env, cargo_features, network_pin, postgres_admin_url):
        index_root = Path(harness_env[promote.PIPELINE_INDEX_ROOT_VAR])
        self.hook_calls.append(
            {
                "run_id": run.run_id,
                "harness_env": dict(harness_env),
                "cargo_features": tuple(cargo_features),
                "network_pin": Path(network_pin),
                "postgres_admin_url": postgres_admin_url,
                "index_root_left_over": index_root.exists(),
                "attestations_left_over": sorted(path.name for path in run.results_dir.glob("*.attestation.json")),
            }
        )
        for check_id in promote.package_check_ids():
            self._write_production_result(
                run.run_id, check_id, package=self.hook_package, evidence=self.hook_evidence.get(check_id)
            )

    def _package_checks(self, run_id, *extra, env_file=None):
        argv = ["promote", "package-checks", "--run-id", run_id, "--env-file", str(env_file or self.env_path)]
        with mock.patch.object(pipeline, "run_package_checks", self._fake_package_checks):
            return self._main([*argv, *extra])

    def test_package_checks_run_the_production_assembly_from_the_env_file(self):
        run_id = self._init()
        self.assertEqual(self._package_checks(run_id), 0, self.stderr.getvalue())
        [call] = self.hook_calls
        run_dir = (self._runs_dir() / run_id).resolve()
        env = call["harness_env"]
        self.assertEqual(call["run_id"], run_id)
        self.assertEqual(
            set(env),
            {
                "TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY",
                "TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH",
                "TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH",
                "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT",
                "TRACE_COMMONS_NEAR_AI_BASE_URL",
                "TRACE_COMMONS_NEAR_AI_API_KEY",
                "TRACE_COMMONS_NEAR_AI_MODEL",
                "TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS",
                "TRACE_COMMONS_EMBEDDER_CACHE_DIR",
                "TRACE_COMMONS_VECTOR_INDEX_DIM",
            },
            "only the allowlisted variables of the env file reach the harness",
        )
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY"], "production")
        self.assertEqual(env["TRACE_COMMONS_NEAR_AI_API_KEY"], "near-ai-test-key")
        self.assertEqual(Path(env["TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH"]).resolve(), run_dir / promote.PACKAGE_FILE)
        self.assertEqual(
            Path(env["TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH"]).resolve(), run_dir / promote.TRUSTED_KEY_FILE
        )
        self.assertEqual(Path(env["TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT"]).resolve(), run_dir / promote.INDEX_DIR)
        self.assertEqual(call["cargo_features"], _PROMOTE_FEATURES)
        self.assertEqual(call["network_pin"].resolve(), self.pin_path.resolve())
        self.assertIsNone(call["postgres_admin_url"])
        for check_id in promote.package_check_ids():
            result, evidence = self._result(run_id, check_id)
            self.assertEqual(
                (result["package_hash"], result["configuration_digest"], result["dependency_digest"]),
                _package_triple(self.signed),
            )
            self.assertEqual(evidence["harness_assembly"], "production")
        output = self.stdout.getvalue()
        self.assertIn("PipelinePromotePackageChecksOK:", output)
        self.assertNotIn(str(self.root), output)
        self.assertNotIn("near-ai-test-key", output + self.stderr.getvalue())

    def test_package_checks_pass_the_admin_url_through(self):
        run_id = self._init()
        self.assertEqual(self._package_checks(run_id, "--postgres-admin-url", _ADMIN_URL), 0, self.stderr.getvalue())
        self.assertEqual(self.hook_calls[0]["postgres_admin_url"], _ADMIN_URL)

    def test_package_checks_refuse_an_incomplete_env_file_before_anything_runs(self):
        run_id = self._init()
        complete = self.env_path.read_text()
        cases = {
            "promote_env_file_incomplete:no key": complete.replace('TRACE_COMMONS_NEAR_AI_API_KEY="near-ai-test-key"\n', ""),
            "promote_env_file_incomplete:blank key": complete.replace('"near-ai-test-key"', '""'),
            "promote_env_file_incomplete:no model": complete.replace(
                "TRACE_COMMONS_NEAR_AI_MODEL=Qwen/Qwen3.6-35B-A3B-FP8\n", ""
            ),
            "promote_env_file_incomplete:no endpoint": complete.replace(
                "TRACE_COMMONS_NEAR_AI_BASE_URL=https://near-ai.invalid/v1\n", ""
            ),
            "env_file_invalid:not an assignment": complete + "export TRACE_COMMONS_NEAR_AI_MODEL\n",
        }
        for case, text in cases.items():
            with self.subTest(case=case):
                env_file = self.root / "inputs" / "case.env"
                env_file.write_text(text)
                self.stderr.seek(0)
                self.stderr.truncate()
                self.assertEqual(self._package_checks(run_id, env_file=env_file), 1)
                self.assertEqual(self._failure(), f"PipelineFailure: {case.split(':')[0]}")
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._package_checks(run_id, env_file=self.root / "inputs" / "absent.env"), 1)
        self.assertEqual(self._failure(), "PipelineFailure: env_file_unreadable")
        self.assertEqual(self.hook_calls, [])
        self.assertEqual(list(self._results_dir(run_id).iterdir()), [])

    def test_package_checks_take_only_the_committed_network_pin(self):
        run_id = self._init()
        committed = self.pin_path.read_text()

        def refused(label, *extra):
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._package_checks(run_id, *extra), 1)
            self.assertEqual(self._failure(), f"PipelineFailure: {label}")

        # The same bytes elsewhere are the committed pin.
        copy = self.root / "scratch" / "pin-network.json"
        copy.parent.mkdir()
        copy.write_text(committed)
        self.assertEqual(self._package_checks(run_id, "--pin", str(copy)), 0, self.stderr.getvalue())
        self.assertEqual(self.hook_calls[-1]["network_pin"].resolve(), self.pin_path.resolve())
        calls = len(self.hook_calls)
        # A pin for another revision, or none at the path given.
        other = dict(_network_pin(self.manifest), revision="f" * 40)
        copy.write_text(json.dumps(other))
        refused("hf_network_pin_not_committed", "--pin", str(copy))
        refused("hf_network_pin_not_committed", "--pin", str(self.root / "absent.json"))
        # The committed pin itself must be a network pin, and must exist.
        self.pin_path.write_text(json.dumps({**_network_pin(self.manifest), "local_jsonl_dir": "fixtures"}))
        refused("hf_network_pin_has_local_dir")
        self.pin_path.unlink()
        refused("hf_network_pin_missing")
        refused("hf_network_pin_missing", "--pin", str(copy))
        self.assertEqual(len(self.hook_calls), calls)

    def test_package_checks_refuse_a_result_not_from_the_production_assembly(self):
        run_id = self._init()
        for check_id in promote.package_check_ids():
            for evidence in ({"observed": check_id}, {"observed": check_id, "harness_assembly": "reference"}):
                with self.subTest(check_id=check_id, evidence=evidence):
                    self.hook_evidence = {check_id: evidence}
                    self.stderr.seek(0)
                    self.stderr.truncate()
                    self.assertEqual(self._package_checks(run_id), 1)
                    self.assertEqual(
                        self._failure(), f"PipelineFailure: promote_harness_assembly_not_production:{check_id}"
                    )

    def test_package_checks_refuse_another_package(self):
        run_id = self._init()
        self.hook_package = (_fake_hash("p"), _fake_hash("c"), _fake_hash("d"))
        self.assertEqual(self._package_checks(run_id), 1)
        self.assertIn(
            self._failure(),
            ("PipelineFailure: qualification_evidence_mixed_package", "PipelineFailure: promote_package_mismatch"),
        )

    def test_a_rerun_starts_from_empty_indexes_and_drops_attestations(self):
        run_id = self._init()
        run_dir = self._runs_dir() / run_id
        (run_dir / promote.INDEX_DIR / "pipeline_restore_drill" / "resume").mkdir(parents=True)
        (run_dir / promote.INDEX_DIR / "pipeline_restore_drill" / "resume" / "stale.usearch").write_bytes(b"stale")
        (self._results_dir(run_id) / "pipeline_hf_network_canary.attestation.json").write_text("{}")
        self.assertEqual(self._package_checks(run_id), 0, self.stderr.getvalue())
        [call] = self.hook_calls
        self.assertFalse(call["index_root_left_over"])
        self.assertEqual(call["attestations_left_over"], [])

    def test_promote_sign_refuses_a_package_check_not_from_the_production_assembly(self):
        run_id = self._init()
        production = promote.production_check_ids()
        for check_id in production:
            self._write_production_result(run_id, check_id)
        [first] = promote.package_check_ids()[:1]
        self._write_production_result(run_id, first, evidence={"observed": first})
        key = self.root / "inputs" / "check-key.pk8"
        key.write_bytes(b"fake pkcs8")
        code = self._main(
            ["promote", "sign", "--run-id", run_id, "--signing-key", str(key), "--signing-key-id", "operator_check_key"]
        )
        self.assertEqual(code, 1)
        self.assertEqual(self._failure(), f"PipelineFailure: promote_harness_assembly_not_production:{first}")
        self.assertEqual([call for call in self.calls if call[0] == "cargo"], [])


class HfNetworkPinTests(_PromoteCase):
    def _record(self, output, revision=_HEX40):
        return self._main(["hf-pin", "record", "--revision", revision, "--output", str(output)])

    def test_hf_pin_network_has_no_local_dir(self):
        committed = environment.ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-network.json"
        if committed.exists():
            pin = json.loads(committed.read_text())
            self.assertNotIn("local_jsonl_dir", pin)
            self.assertEqual(pin["schema"], corpus.PIN_SCHEMA)
        output = self.root / "pin-network.json"
        self.assertEqual(self._record(output), 0, self.stderr.getvalue())
        self.assertNotIn("local_jsonl_dir", json.loads(output.read_text()))

        # The canary refuses a committed pin that names a local directory.
        run_id = self._init()
        local = json.loads(output.read_text())
        local["local_jsonl_dir"] = "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl"
        committed_pin = self.root / promote.HF_NETWORK_PIN_PATH
        committed_pin.parent.mkdir(parents=True, exist_ok=True)
        committed_pin.write_text(json.dumps(local))
        self.assertEqual(self._main(["promote", "hf-canary", "--run-id", run_id]), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_network_pin_has_local_dir")

    def test_hf_pin_record_writes_every_digest(self):
        output = self.root / "pins" / "pin-network.json"
        self.assertEqual(self._record(output), 0, self.stderr.getvalue())
        pin = json.loads(output.read_text())
        local = json.loads(promote.HF_LOCAL_PIN.read_text())
        expected = {key: value for key, value in local.items() if key != "local_jsonl_dir"}
        expected["revision"] = _HEX40
        for field in promote.HF_PIN_DIGEST_FIELDS:
            expected[field] = self.manifest[field]
        self.assertEqual(pin, expected)
        self.assertEqual(
            set(promote.HF_PIN_DIGEST_FIELDS),
            {"source_digest", "configuration_digest", "order_digest", "bootstrap_corpus_digest", "holdout_corpus_digest"},
        )
        # The download went to a cache inside the run directory, with no
        # expected digests (a draft pin has none).
        [(_, step, fields, output_dir, cache_dir)] = [call for call in self.calls if call[0] == "export"]
        self.assertEqual(step, "hf_pin_record")
        self.assertNotIn("source_digest", fields)
        self.assertNotIn("local_jsonl_dir", fields)
        [run_id] = [name for name in self._runs()]
        self.assertTrue(cache_dir.is_relative_to(self._runs_dir() / run_id))
        self.assertTrue(output_dir.is_relative_to(self._runs_dir() / run_id))

        # Never overwritten; never from a revision that is not a commit.
        before = output.read_bytes()
        self.assertEqual(self._record(output), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_pin_output_exists")
        self.assertEqual(output.read_bytes(), before)
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._record(self.root / "other.json", revision="main"), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_pin_revision_invalid")

        # A manifest that is not the pinned source is refused.
        self.stderr.seek(0)
        self.stderr.truncate()
        self.manifest = _hf_network_manifest(source={**self.manifest["source"], "revision": "f" * 40})
        self.assertEqual(self._record(self.root / "third.json"), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_manifest_source_mismatch")
        self.assertFalse((self.root / "third.json").exists())


class HfCanaryTests(_PromoteCase):
    def setUp(self):
        super().setUp()
        # The committed pin: where the repository keeps it, in the scratch root.
        self.pin_path = self.root / promote.HF_NETWORK_PIN_PATH
        self.pin_path.parent.mkdir(parents=True, exist_ok=True)
        local = json.loads(promote.HF_LOCAL_PIN.read_text())
        pin = {key: value for key, value in local.items() if key != "local_jsonl_dir"}
        pin["revision"] = _HEX40
        for field in promote.HF_PIN_DIGEST_FIELDS:
            pin[field] = self.manifest[field]
        self.pin_path.write_text(json.dumps(pin))
        self.pin = pin

    def _canary(self, run_id):
        return self._main(["promote", "hf-canary", "--run-id", run_id, "--pin", str(self.pin_path)])

    def test_hf_canary_evidence_shape(self):
        run_id = self._init()
        self.assertEqual(self._canary(run_id), 0, self.stderr.getvalue())
        result, evidence = self._result(run_id, promote.HF_CANARY_CHECK_ID)
        self.assertEqual(
            evidence,
            {
                "schema": "trace_commons.pipeline_hf_network_canary.v1",
                "repository_owner": "jedisct1",
                "repository_name": "security-audits",
                "revision": _HEX40,
                "pin_hash": _digest(self.pin_path.read_bytes()),
                "source_digest": self.manifest["source_digest"],
                "order_digest": self.manifest["order_digest"],
                "bootstrap_corpus_digest": self.manifest["bootstrap_corpus_digest"],
                "holdout_corpus_digest": self.manifest["holdout_corpus_digest"],
                "downloaded_file_count": 2,
                "cache_dir_inside_run": True,
            },
        )
        results.validate_evidence(evidence)
        package_hash, configuration_digest, dependency_digest = _package_triple(self.signed)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["run_id"], run_id)
        self.assertEqual(result["code_revision_hash"], self.tree)
        self.assertEqual(result["safe_blockers"], [])
        self.assertEqual(
            (result["package_hash"], result["configuration_digest"], result["dependency_digest"]),
            (package_hash, configuration_digest, dependency_digest),
        )
        self.assertEqual(result["evidence_hash"], _digest(results.canonical(evidence)))
        # The results load as a Rust result does.
        run = environment.Run(run_id, datetime.now(timezone.utc) - timedelta(minutes=5), self._runs_dir() / run_id, self.tree)
        loaded = results.load_results(run)
        results.require_current_pass_results(run, loaded, {promote.HF_CANARY_CHECK_ID: checks.CheckSpec(promote.HF_CANARY_CHECK_ID, True)})
        # The download used a fresh cache inside the production run.
        [(_, step, fields, output_dir, cache_dir)] = [call for call in self.calls if call[0] == "export"]
        self.assertTrue(cache_dir.is_relative_to(self._runs_dir() / run_id))
        self.assertNotIn("local_jsonl_dir", fields)
        self.assertIn(f"PipelinePromoteCheckOK: check={promote.HF_CANARY_CHECK_ID} status=pass", self.stdout.getvalue())

        # A re-run replaces its own result, in the same run directory.
        self.assertEqual(self._canary(run_id), 0, self.stderr.getvalue())
        self.assertEqual(sorted(path.name for path in self._results_dir(run_id).iterdir()),
                         [f"{promote.HF_CANARY_CHECK_ID}.evidence.json", f"{promote.HF_CANARY_CHECK_ID}.result.json"])

    def test_hf_canary_fails_on_digest_mismatch(self):
        run_id = self._init()
        for field in promote.HF_PIN_DIGEST_FIELDS:
            self.manifest = _hf_network_manifest(**{field: _fake_hash(f"moved-{field}")})
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._canary(run_id), 1, field)
            self.assertEqual(self._failure(), f"PipelineFailure: check_result_failed:{promote.HF_CANARY_CHECK_ID}")
            result, evidence = self._result(run_id, promote.HF_CANARY_CHECK_ID)
            self.assertEqual(result["status"], "fail", field)
            self.assertEqual(result["safe_blockers"], [f"hf_pin_digest_mismatch_{field}"], field)
            results.validate_evidence(evidence)

    def test_hf_canary_fails_when_nothing_was_downloaded(self):
        run_id = self._init()
        self.downloaded = 0
        self.assertEqual(self._canary(run_id), 1)
        result, _ = self._result(run_id, promote.HF_CANARY_CHECK_ID)
        self.assertEqual((result["status"], result["safe_blockers"]), ("fail", ["hf_network_download_missing"]))

    def test_hf_canary_accepts_only_the_committed_pin(self):
        """Review of #1293: a pin `hf-pin record` wrote for another revision,
        outside the tree, matches its own fresh download in every digest. The
        code revision does not cover it, so the canary refuses any pin whose
        bytes are not the committed `pin-network.json`'s, before it downloads."""
        run_id = self._init()
        other = dict(self.pin, revision="f" * 40)
        elsewhere = self.root / "elsewhere" / "pin-network.json"
        elsewhere.parent.mkdir()
        elsewhere.write_text(json.dumps(other))
        # Every digest of the other revision's download matches its pin.
        self.manifest = _hf_network_manifest(source={**self.manifest["source"], "revision": "f" * 40})
        self.assertEqual(self._main(["promote", "hf-canary", "--run-id", run_id, "--pin", str(elsewhere)]), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_network_pin_not_committed")
        self.assertEqual([call for call in self.calls if call[0] == "export"], [])
        self.assertFalse((self._results_dir(run_id) / f"{promote.HF_CANARY_CHECK_ID}.result.json").exists())

        # A copy of the committed pin's bytes, wherever it is, is the
        # committed pin.
        self.manifest = _hf_network_manifest()
        copy = self.root / "elsewhere" / "copy.json"
        copy.write_bytes(self.pin_path.read_bytes())
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._main(["promote", "hf-canary", "--run-id", run_id, "--pin", str(copy)]), 0,
                         self.stderr.getvalue())
        _, evidence = self._result(run_id, promote.HF_CANARY_CHECK_ID)
        self.assertEqual(evidence["pin_hash"], _digest(self.pin_path.read_bytes()))

        # With no `--pin`, the committed pin is the one used.
        self.assertEqual(self._main(["promote", "hf-canary", "--run-id", run_id]), 0, self.stderr.getvalue())

        # No committed pin: nothing is accepted in its place.
        self.pin_path.unlink()
        for argv in ([], ["--pin", str(copy)]):
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._main(["promote", "hf-canary", "--run-id", run_id, *argv]), 1, argv)
            self.assertEqual(self._failure(), "PipelineFailure: hf_network_pin_missing", argv)

    def test_hf_canary_refuses_a_pin_without_every_digest(self):
        run_id = self._init()
        del self.pin["holdout_corpus_digest"]
        self.pin_path.write_text(json.dumps(self.pin))
        self.assertEqual(self._canary(run_id), 1)
        self.assertEqual(self._failure(), "PipelineFailure: hf_network_pin_invalid")
        self.assertEqual([call for call in self.calls if call[0] == "export"], [])


class RemoteRestoreTests(_PromoteCase):
    def _restore(self, run_id, source="tracecommons-artifacts/live", scratch="tracecommons-restore-scratch/drill", **kwargs):
        return self._main(
            ["promote", "remote-restore", "--run-id", run_id, "--source-store", source, "--scratch-store", scratch], **kwargs
        )

    def test_remote_restore_evidence_shape(self):
        run_id = self._init()
        self.assertEqual(self._restore(run_id), 0, self.stderr.getvalue())
        result, evidence = self._result(run_id, promote.REMOTE_RESTORE_CHECK_ID)
        self.assertEqual(
            evidence,
            {
                "schema": "trace_commons.pipeline_remote_restore.v1",
                "object_store_kind": "gcs",
                "source_store_name_hash": _digest(b"tracecommons-artifacts/live"),
                "scratch_store_name_hash": _digest(b"tracecommons-restore-scratch/drill"),
                "object_count": 42,
                "artifact_fingerprint": self.report["artifact_fingerprint"],
                "restored_artifact_fingerprint": self.report["restored_artifact_fingerprint"],
                "versioning_enabled": True,
                "kek_unwrap_verified_count": 42,
                "database_fingerprint": self.report["seed_database_fingerprint"],
                "pending_runs_resumed": 1,
                "duplicate_effects": 0,
            },
        )
        results.validate_evidence(evidence)
        self.assertEqual((result["status"], result["safe_blockers"]), ("pass", []))
        self.assertEqual(
            (result["package_hash"], result["configuration_digest"], result["dependency_digest"]),
            _package_triple(self.signed),
        )
        [(step, source, scratch, report_path)] = self.harness_calls
        self.assertEqual((source, scratch), ("tracecommons-artifacts/live", "tracecommons-restore-scratch/drill"))
        self.assertTrue(report_path.is_relative_to(self._runs_dir() / run_id))
        for text in (self.stdout.getvalue(), self.stderr.getvalue(), json.dumps(result), json.dumps(evidence)):
            self.assertNotIn("tracecommons-artifacts", text)
            self.assertNotIn("tracecommons-restore-scratch", text)

    def test_remote_restore_evidence_rejects_url_like_values(self):
        run_id = self._init()
        # An argument that looks like a URL never reaches argparse.
        with self.assertRaises(errors.ToolingError) as ctx:
            pipeline.parse_args(
                ["promote", "remote-restore", "--run-id", run_id, "--source-store", "live",
                 "--scratch-store", "gs://tracecommons-restore-scratch"]
            )
        self.assertEqual(str(ctx.exception), "argument_looks_like_url")
        # A store name outside the bucket/prefix shape is refused too.
        for name in ("Bucket With Spaces", "bucket/../live", "x", "bucket//double", "bucket/"):
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._restore(run_id, scratch=name), 1, name)
            self.assertEqual(self._failure(), "PipelineFailure: remote_restore_store_name_invalid", name)
        # A report value that looks like a URL is refused, and no result is written.
        for overrides in (
            {"object_store_kind": "gs://tracecommons-artifacts"},
            {"artifact_fingerprint": "https://storage.googleapis.com/x"},
            {"extra": "value"},
            {"object_count": 1.5},
        ):
            self.report = _remote_report(**overrides)
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._restore(run_id), 1, overrides)
            self.assertEqual(self._failure(), "PipelineFailure: remote_restore_report_invalid", overrides)
            self.assertEqual(list(self._results_dir(run_id).iterdir()), [], overrides)

    def test_remote_restore_fails_on_fingerprint_mismatch(self):
        run_id = self._init()
        for overrides, blocker in (
            ({"restored_artifact_fingerprint": _fake_hash("other")}, "remote_restore_artifact_fingerprint_mismatch"),
            ({"kek_unwrap_verified_count": 41}, "remote_restore_unwrap_count_mismatch"),
            ({"versioning_enabled": False}, "remote_restore_versioning_disabled"),
            ({"resumed_database_fingerprint": _fake_hash("other")}, "remote_restore_resume_mismatch"),
            ({"pending_runs_resumed": 0}, "remote_restore_resume_mismatch"),
            ({"duplicate_effects": 1}, "remote_restore_resume_mismatch"),
            ({"object_store_kind": "file_system"}, "remote_restore_store_not_remote"),
            ({"object_count": 0, "kek_unwrap_verified_count": 0}, "remote_restore_empty"),
        ):
            self.report = _remote_report(**overrides)
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._restore(run_id), 1, overrides)
            self.assertEqual(self._failure(), f"PipelineFailure: check_result_failed:{promote.REMOTE_RESTORE_CHECK_ID}")
            result, evidence = self._result(run_id, promote.REMOTE_RESTORE_CHECK_ID)
            self.assertEqual(result["status"], "fail", overrides)
            self.assertIn(blocker, result["safe_blockers"], overrides)
            results.validate_evidence(evidence)

    def test_remote_restore_refuses_the_live_prefix_as_target(self):
        run_id = self._init()
        for source, scratch in (
            ("tracecommons-artifacts", "tracecommons-artifacts"),
            ("tracecommons-artifacts", "tracecommons-artifacts/scratch"),
            ("tracecommons-artifacts/live", "tracecommons-artifacts"),
            ("tracecommons-artifacts/live", "tracecommons-artifacts/live/drill"),
        ):
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._restore(run_id, source=source, scratch=scratch), 1, (source, scratch))
            self.assertEqual(self._failure(), "PipelineFailure: remote_restore_scratch_overlaps_live_store")
        self.assertEqual(self.harness_calls, [])
        # A sibling prefix in the same bucket is not the live prefix.
        self.assertEqual(
            self._restore(run_id, source="tracecommons-artifacts/live", scratch="tracecommons-artifacts/live-drill"),
            0,
            self.stderr.getvalue(),
        )

    def test_remote_restore_passes_the_postgres_admin_url_to_the_harness(self):
        run_id = self._init()
        self.assertEqual(
            self._main(
                ["promote", "remote-restore", "--run-id", run_id, "--source-store", "tracecommons-artifacts",
                 "--scratch-store", "tracecommons-restore-scratch",
                 "--postgres-admin-url", "postgres://trace@127.0.0.1:5432/postgres"]
            ),
            0,
            self.stderr.getvalue(),
        )
        [(_, _, _, _, admin_url)] = self.harness_admin_urls
        self.assertEqual(admin_url, "postgres://trace@127.0.0.1:5432/postgres")


class _FakeScenario:
    def __init__(self, log):
        self.log = log
        self.runtime_url = "postgres://trace@127.0.0.1:5432/pipeline_remote_restore"
        self.pilot_database = "pipeline_remote_restore_pilot"
        self.restored_database = "pipeline_remote_restore_restored"
        self.restored_url = "postgres://trace@127.0.0.1:5432/pipeline_remote_restore_restored"

    def committed_transactions(self, database):
        self.log.append(("committed", database))
        return 100


class _FakeEnvironment:
    """`Environment`'s surface the remote restore harness uses, recording
    each call in order."""

    def __init__(self, log):
        self.log = log

    def __call__(self, run, *, postgres_admin_url=None):
        self.log.append(("environment", postgres_admin_url))
        return self

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.log.append(("environment_closed",))
        return False

    def scenario(self, name):
        self.log.append(("scenario", name))
        return _FakeScenario(self.log)

    def dump(self, database, path):
        self.log.append(("dump", database))
        Path(path).write_bytes(b"dump")

    def create_database(self, database):
        self.log.append(("create_database", database))

    def restore(self, path, database):
        self.log.append(("restore", database))


class RemoteRestoreHarnessTests(_PromoteCase):
    """The harness itself (`run_remote_restore_harness`): the seed, the
    database restore, the copy, and the resume, with cargo and PostgreSQL
    replaced."""

    def setUp(self):
        super().setUp()
        self.log = []
        self.seed_artifacts = _fake_hash("artifacts")
        self.copy = {
            "schema": "trace_commons.pipeline_remote_restore_copy.v1",
            "object_store_kind": "gcs",
            "object_count": 7,
            "artifact_fingerprint": self.seed_artifacts,
            "restored_artifact_fingerprint": self.seed_artifacts,
            "kek_unwrap_verified_count": 7,
            "versioning_enabled": True,
        }
        self.resume = {
            "schema": "trace_commons.pipeline_remote_restore_resume.v1",
            "database_fingerprint": _fake_hash("database"),
            "artifact_fingerprint": self.seed_artifacts,
            "pending_runs_resumed": 1,
            "duplicate_effects": 0,
        }

    def _seed_fingerprint(self):
        hashes = ("database_fingerprint", "artifact_fingerprint", "index_entry_set_hash", "pending_run_id_hash",
                  "runtime_privilege_set_hash", "tenant_fingerprint", "rls_policy_set_hash", "rls_flag_set_hash")
        value = {"schema": "trace_commons.pipeline_restore_fingerprint.v1"}
        value.update({key: _fake_hash(key) for key in hashes})
        value["database_fingerprint"] = _fake_hash("database")
        value["artifact_fingerprint"] = self.seed_artifacts
        value.update({key: 3 for key in ("adapter_request_count", "completed_settlement_count",
                                          "completed_credit_event_count", "rls_table_count", "rls_policy_count",
                                          "rls_flag_table_count", "runtime_privilege_count", "tenant_count",
                                          "audit_event_count")})
        return value

    def _fake_harness_cargo(self, run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
        self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
        self.log.append(("cargo", step))
        if step == "remote_restore_seed":
            Path(env["TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH"]).write_text(json.dumps(self._seed_fingerprint()))
        elif step == "remote_restore_copy":
            Path(env["TRACE_COMMONS_PIPELINE_REMOTE_COPY_REPORT_PATH"]).write_text(json.dumps(self.copy))
        elif step == "remote_restore_resume":
            Path(env["TRACE_COMMONS_PIPELINE_REMOTE_RESUME_REPORT_PATH"]).write_text(json.dumps(self.resume))

    def _restore(self, run_id, *, extra_environ=None):
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(cargo, "cargo_test", self._fake_harness_cargo))
            stack.enter_context(mock.patch.object(promote, "Environment", _FakeEnvironment(self.log)))
            environ = stack.enter_context(mock.patch.dict(os.environ))
            for key in ("TRACE_COMMONS_KEK_PROVIDER", "TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME"):
                environ.pop(key, None)
            environ.update(extra_environ or {})
            return self._main(
                ["promote", "remote-restore", "--run-id", run_id, "--source-store", "tracecommons-artifacts",
                 "--scratch-store", "tracecommons-restore-scratch/drill"],
                harness=False,
            )

    def _cargo(self):
        return {call[1]: call for call in self.calls if call[0] == "cargo"}

    def test_the_harness_seeds_restores_copies_and_resumes_in_order(self):
        run_id = self._init()
        self.assertEqual(self._restore(run_id), 0, self.stderr.getvalue())
        steps = [entry for entry in self.log if entry[0] in ("cargo", "dump", "create_database", "restore")]
        self.assertEqual(
            steps,
            [
                ("cargo", "remote_restore_seed"),
                ("dump", "pipeline_remote_restore_pilot"),
                ("create_database", "pipeline_remote_restore_restored"),
                ("restore", "pipeline_remote_restore_restored"),
                ("cargo", "remote_restore_copy"),
                ("cargo", "remote_restore_resume"),
            ],
        )
        self.assertEqual(self.log[-1], ("environment_closed",))
        calls = self._cargo()
        self.assertEqual(calls["remote_restore_seed"][3], "tests::pipeline_restore_pg_tests::pipeline_restore_seed")
        self.assertEqual(calls["remote_restore_copy"][3], promote.REMOTE_RESTORE_RUN)
        self.assertEqual(promote.REMOTE_RESTORE_RUN, "tests::pipeline_restore_pg_tests::pipeline_remote_restore_run")
        self.assertEqual(calls["remote_restore_resume"][3], "tests::pipeline_restore_pg_tests::pipeline_restore_resume")
        for _, step, cargo_args, _, env, exact, ignored in calls.values():
            self.assertEqual(cargo_args, promote.PROMOTE_CARGO_ARGS, step)
            self.assertTrue(exact and ignored, step)
            self.assertNotIn("TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT", env, step)
            self.assertNotIn("TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR", env, step)
            self.assertEqual(env["TRACE_COMMONS_PIPELINE_REMOTE_STORE_KIND"], "gcs", step)
        seed_env = calls["remote_restore_seed"][4]
        copy_env = calls["remote_restore_copy"][4]
        resume_env = calls["remote_restore_resume"][4]
        # The seed writes into the live store, the resume reads the scratch
        # store; the copy names both. One namespace, fresh, and one key.
        self.assertEqual(seed_env["TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE"], "tracecommons-artifacts")
        self.assertEqual(resume_env["TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE"], "tracecommons-restore-scratch/drill")
        self.assertEqual(copy_env["TRACE_COMMONS_PIPELINE_REMOTE_SOURCE_STORE"], "tracecommons-artifacts")
        self.assertEqual(copy_env["TRACE_COMMONS_PIPELINE_REMOTE_SCRATCH_STORE"], "tracecommons-restore-scratch/drill")
        namespaces = {env["TRACE_COMMONS_PIPELINE_REMOTE_NAMESPACE"] for env in (seed_env, copy_env, resume_env)}
        [namespace] = namespaces
        self.assertRegex(namespace, rf"^pipeline-remote-restore-{run_id}-[0-9a-f]{{8}}$")
        keys = {env["TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX"] for env in (seed_env, copy_env, resume_env)}
        [key] = keys
        self.assertRegex(key, r"^[0-9a-f]{64}$")
        self.assertEqual(seed_env["TRACE_COMMONS_PG_TEST_DATABASE_URL"], _FakeScenario([]).runtime_url)
        self.assertEqual(resume_env["TRACE_COMMONS_PG_TEST_DATABASE_URL"], _FakeScenario([]).restored_url)
        self.assertNotIn("TRACE_COMMONS_PG_TEST_DATABASE_URL", copy_env)
        # The result, from the measurements, names the store only by hash.
        result, evidence = self._result(run_id, promote.REMOTE_RESTORE_CHECK_ID)
        self.assertEqual((result["status"], result["safe_blockers"]), ("pass", []))
        self.assertEqual(
            evidence,
            {
                "schema": "trace_commons.pipeline_remote_restore.v1",
                "object_store_kind": "gcs",
                "source_store_name_hash": _digest(b"tracecommons-artifacts"),
                "scratch_store_name_hash": _digest(b"tracecommons-restore-scratch/drill"),
                "object_count": 7,
                "artifact_fingerprint": self.seed_artifacts,
                "restored_artifact_fingerprint": self.seed_artifacts,
                "versioning_enabled": True,
                "kek_unwrap_verified_count": 7,
                "database_fingerprint": _fake_hash("database"),
                "pending_runs_resumed": 1,
                "duplicate_effects": 0,
            },
        )
        # Nothing the harness wrote stays behind but its report: no dump, no key.
        work = self._runs_dir() / run_id / "remote-restore"
        self.assertEqual(sorted(path.name for path in work.iterdir()), ["remote-restore-report.json"])
        for text in (self.stdout.getvalue(), self.stderr.getvalue()):
            self.assertNotIn(key, text)
            self.assertNotIn("tracecommons", text)

    def test_the_harness_forwards_only_the_key_wrapper_selection(self):
        run_id = self._init()
        self.assertEqual(
            self._restore(run_id, extra_environ={
                "TRACE_COMMONS_KEK_PROVIDER": "gcp_cloud_kms",
                "TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME": "projects/p/locations/l/keyRings/r/cryptoKeys/k",
                "GOOGLE_APPLICATION_CREDENTIALS": "/secret/credentials.json",
                "TRACE_COMMONS_OBJECT_STORE_BUCKET": "tracecommons-artifacts",
            }),
            0,
            self.stderr.getvalue(),
        )
        for _, step, _, _, env, _, _ in self._cargo().values():
            self.assertEqual(env["TRACE_COMMONS_KEK_PROVIDER"], "gcp_cloud_kms", step)
            self.assertEqual(
                env["TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME"], "projects/p/locations/l/keyRings/r/cryptoKeys/k", step
            )
            self.assertNotIn("GOOGLE_APPLICATION_CREDENTIALS", env, step)
            self.assertNotIn("TRACE_COMMONS_OBJECT_STORE_BUCKET", env, step)
        self.assertNotIn("keyRings", self.stdout.getvalue() + self.stderr.getvalue())
        # Unset, nothing is forwarded: the children fall back to the local key.
        self.calls.clear()
        self.assertEqual(self._restore(run_id), 0, self.stderr.getvalue())
        for _, step, _, _, env, _, _ in self._cargo().values():
            self.assertNotIn("TRACE_COMMONS_KEK_PROVIDER", env, step)
            self.assertNotIn("TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME", env, step)

    def test_the_harness_refuses_a_source_that_moved_after_the_seed(self):
        run_id = self._init()
        self.copy["artifact_fingerprint"] = _fake_hash("moved")
        self.assertEqual(self._restore(run_id), 1)
        self.assertEqual(self._failure(), "PipelineFailure: remote_restore_source_changed")
        self.assertEqual(list(self._results_dir(run_id).iterdir()), [])

    def test_the_harness_refuses_a_resume_on_other_artifacts(self):
        run_id = self._init()
        self.resume["artifact_fingerprint"] = _fake_hash("other")
        self.assertEqual(self._restore(run_id), 1)
        self.assertEqual(self._failure(), "PipelineFailure: remote_restore_resume_report_invalid")
        self.assertEqual(list(self._results_dir(run_id).iterdir()), [])

    def test_the_harness_refuses_malformed_measurements(self):
        run_id = self._init()
        for target, overrides, label in (
            ("copy", {"extra": 1}, "remote_restore_copy_report_invalid"),
            ("copy", {"object_count": -1}, "remote_restore_copy_report_invalid"),
            ("copy", {"schema": "trace_commons.pipeline_remote_restore_copy.v0"}, "remote_restore_copy_report_invalid"),
            ("copy", {"versioning_enabled": 1}, "remote_restore_copy_report_invalid"),
            ("copy", {"object_store_kind": "gs://bucket"}, "remote_restore_copy_report_invalid"),
            ("resume", {"pending_runs_resumed": True}, "remote_restore_resume_report_invalid"),
            ("resume", {"database_fingerprint": "sha256:short"}, "remote_restore_resume_report_invalid"),
        ):
            saved = (dict(self.copy), dict(self.resume))
            getattr(self, target).update(overrides)
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._restore(run_id), 1, (target, overrides))
            self.assertEqual(self._failure(), f"PipelineFailure: {label}", (target, overrides))
            self.assertEqual(list(self._results_dir(run_id).iterdir()), [], (target, overrides))
            self.copy, self.resume = saved

    def test_a_failing_measurement_is_a_failing_result(self):
        run_id = self._init()
        self.copy["versioning_enabled"] = False
        self.copy["object_store_kind"] = "gcs_directory_double"
        self.assertEqual(self._restore(run_id), 1)
        result, _ = self._result(run_id, promote.REMOTE_RESTORE_CHECK_ID)
        self.assertEqual(result["status"], "fail")
        self.assertEqual(
            result["safe_blockers"], ["remote_restore_store_not_remote", "remote_restore_versioning_disabled"]
        )


class PromoteAdaptersTests(_PromoteCase):
    def test_promote_adapters_refuses_another_run_or_revision(self):
        run_id = self._init()
        check_id = promote.ADAPTERS_CHECK_ID
        self.assertEqual(self._main(["promote", "adapters", "--run-id", run_id]), 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_adapters_result_missing")
        for overrides in (
            {"run_id": "q99999999"},
            {"code_revision_hash": _fake_hash("deployed elsewhere")},
            {"package": (_fake_hash("p"), _fake_hash("c"), _fake_hash("d"))},
            {"status": "fail"},
            {"safe_blockers": ["synthetic_index"]},
        ):
            package = overrides.pop("package", None)
            self._write_production_result(run_id, check_id, package=package, **overrides)
            self.stderr.seek(0)
            self.stderr.truncate()
            self.assertEqual(self._main(["promote", "adapters", "--run-id", run_id]), 1, overrides)
            self.assertEqual(self._failure(), "PipelineFailure: promote_adapters_result_mismatch", overrides)
        self._write_production_result(run_id, check_id)
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._main(["promote", "adapters", "--run-id", run_id]), 0, self.stderr.getvalue())
        self.assertIn(f"PipelinePromoteCheckOK: check={check_id} status=pass", self.stdout.getvalue())


class PromoteSignTests(_PromoteCase):
    def _sign(self, run_id):
        key = self.root / "inputs" / "check-key.pk8"
        key.write_bytes(b"fake pkcs8")
        return self._main(
            ["promote", "sign", "--run-id", run_id, "--signing-key", str(key), "--signing-key-id", "operator_check_key"]
        )

    def test_promote_sign_signs_exactly_seven(self):
        run_id = self._init()
        for check_id in promote.production_check_ids():
            self._write_production_result(run_id, check_id)
        self.assertEqual(self._sign(run_id), 0, self.stderr.getvalue())
        [call] = [call for call in self.calls if call[0] == "cargo"]
        signed = call[4]["TRACE_COMMONS_PIPELINE_CHECK_IDS"].split(",")
        self.assertEqual(len(signed), 7)
        self.assertEqual(frozenset(signed), frozenset(promote.production_check_ids()))
        attestations = sorted(path.name for path in self._results_dir(run_id).glob("*.attestation.json"))
        self.assertEqual(attestations, sorted(f"{check_id}.attestation.json" for check_id in promote.production_check_ids()))
        self.assertIn("PipelinePromoteSignOK: attested=7", self.stdout.getvalue())

    def test_promote_sign_refuses_a_missing_or_foreign_result(self):
        run_id = self._init()
        production = promote.production_check_ids()
        for check_id in production[1:]:
            self._write_production_result(run_id, check_id)
        self.assertEqual(self._sign(run_id), 1)
        self.assertEqual(self._failure(), f"PipelineFailure: check_result_missing:{production[0]}")
        self._write_production_result(run_id, production[0], package=(_fake_hash("p"), _fake_hash("c"), _fake_hash("d")))
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._sign(run_id), 1)
        self.assertIn(self._failure(), ("PipelineFailure: qualification_evidence_mixed_package", "PipelineFailure: promote_package_mismatch"))
        self.assertEqual(list(self._results_dir(run_id).glob("*.attestation.json")), [])
        self.assertEqual([call for call in self.calls if call[0] == "cargo"], [])


class PromoteAssembleTests(_PromoteCase):
    def _mechanics_run(self, run_id="q0000beef", tree=None):
        """A signed `qualify` run: all 19 of its results and attestations,
        the four package checks naming the reference package."""
        directory = self._runs_dir() / run_id / "results"
        directory.mkdir(parents=True)
        reference = _package_triple(_fake_signed_package("reference"))
        for check_id in sorted(checks.REQUIRED_CHECK_IDS):
            evidence = {"observed": check_id}
            spec = checks.required_specs()[check_id]
            package = reference if spec.digests_required else (None, None, None)
            raw = {
                "schema": results.SCHEMA,
                "run_id": run_id,
                "check_id": check_id,
                "status": "pass",
                "code_revision_hash": tree or self.tree,
                "package_hash": package[0],
                "configuration_digest": package[1],
                "dependency_digest": package[2],
                "observed_at": _iso(datetime.now(timezone.utc)),
                "evidence_hash": _digest(results.canonical(evidence)),
                "safe_blockers": [],
            }
            (directory / f"{check_id}.result.json").write_text(json.dumps(raw))
            (directory / f"{check_id}.evidence.json").write_text(json.dumps(evidence))
            (directory / f"{check_id}.attestation.json").write_text(json.dumps(_attestation_for(raw, "ci_check_key", 86400)))
        return run_id

    def _signed_production_run(self):
        run_id = self._init()
        for check_id in promote.production_check_ids():
            raw = self._write_production_result(run_id, check_id)
            (self._results_dir(run_id) / f"{check_id}.attestation.json").write_text(
                json.dumps(_attestation_for(raw, "operator_check_key", 86400))
            )
        return run_id

    def _assemble(self, run_id, mechanics, output):
        return self._main(["promote", "assemble", "--run-id", run_id, "--mechanics-run-id", mechanics, "--output", str(output)])

    def test_promote_assemble_builds_22_and_refuses_doubles(self):
        mechanics = self._mechanics_run()
        run_id = self._signed_production_run()
        output = self.root / "submission"
        self.assertEqual(self._assemble(run_id, mechanics, output), 0, self.stderr.getvalue())
        names = sorted(path.name for path in output.glob("*.attestation.json"))
        self.assertEqual(len(names), 22)
        self.assertEqual(names, sorted(f"{check_id}.attestation.json" for check_id in _promotion_required_checks()))
        for check_id in promote.mechanics_check_ids():
            self.assertEqual(json.loads((output / f"{check_id}.attestation.json").read_text())["result"]["run_id"], mechanics)
        for check_id in promote.production_check_ids():
            self.assertEqual(json.loads((output / f"{check_id}.attestation.json").read_text())["result"]["run_id"], run_id)
        self.assertEqual(json.loads((output / promote.PACKAGE_FILE).read_text()), self.signed)
        self.assertIn("PipelinePromoteAssembleOK: attestations=22", self.stdout.getvalue())

        # An output that already exists is never written into.
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._assemble(run_id, mechanics, output), 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_assemble_output_exists")

        # A mechanics id signed in the production run is a double.
        mechanics_id = promote.mechanics_check_ids()[0]
        raw = self._write_production_result(run_id, mechanics_id, package=(None, None, None))
        (self._results_dir(run_id) / f"{mechanics_id}.attestation.json").write_text(
            json.dumps(_attestation_for(raw, "operator_check_key", 86400))
        )
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._assemble(run_id, mechanics, self.root / "second"), 1)
        self.assertEqual(self._failure(), f"PipelineFailure: promote_assemble_check_doubled:{mechanics_id}")
        self.assertFalse((self.root / "second").exists())

    def test_promote_assemble_leaves_an_existing_staging_name_alone(self):
        """A directory that happens to have the staging name `.<output>.staging`
        is never removed: the staging directory is a fresh one beside it."""
        mechanics = self._mechanics_run()
        run_id = self._signed_production_run()
        output = self.root / "submission"
        bystander = self.root / ".submission.staging"
        bystander.mkdir()
        (bystander / "keep.txt").write_text("not the tool's")
        self.assertEqual(self._assemble(run_id, mechanics, output), 0, self.stderr.getvalue())
        self.assertEqual((bystander / "keep.txt").read_text(), "not the tool's")
        self.assertEqual(len(list(output.glob("*.attestation.json"))), 22)
        # Nothing of the tool's own staging is left behind.
        self.assertEqual(sorted(path.name for path in self.root.iterdir() if path.name.startswith(".submission")),
                         [".submission.staging"])

        # An output whose parent does not exist yet is still written.
        nested = self.root / "out" / "nested" / "submission"
        self.assertEqual(self._assemble(run_id, mechanics, nested), 0, self.stderr.getvalue())
        self.assertEqual(len(list(nested.glob("*.attestation.json"))), 22)
        self.assertEqual(sorted(path.name for path in nested.parent.iterdir()), ["submission"])

    def test_promote_assemble_refuses_a_missing_or_mismatched_set(self):
        mechanics = self._mechanics_run()
        run_id = self._signed_production_run()
        missing = promote.production_check_ids()[-1]
        (self._results_dir(run_id) / f"{missing}.attestation.json").unlink()
        self.assertEqual(self._assemble(run_id, mechanics, self.root / "a"), 1)
        self.assertEqual(self._failure(), f"PipelineFailure: promote_assemble_check_missing:{missing}")

        run_id = self._signed_production_run()
        mechanics_id = promote.mechanics_check_ids()[0]
        (self._runs_dir() / mechanics / "results" / f"{mechanics_id}.attestation.json").unlink()
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._assemble(run_id, mechanics, self.root / "b"), 1)
        self.assertEqual(self._failure(), f"PipelineFailure: promote_assemble_check_missing:{mechanics_id}")

        # A mechanics run from another revision.
        other = self._mechanics_run("q0000cafe", tree=_fake_hash("another tree"))
        self.stderr.seek(0)
        self.stderr.truncate()
        self.assertEqual(self._assemble(run_id, other, self.root / "c"), 1)
        self.assertEqual(self._failure(), "PipelineFailure: promote_assemble_mixed_revision")
        for name in ("a", "b", "c"):
            self.assertFalse((self.root / name).exists(), name)



# ---------------------------------------------------------------------------
# The pipeline comparison: `pipeline.py compare`.
# ---------------------------------------------------------------------------

_COMPARE_HARNESS = "tests::pipeline_compare_pg_tests::pipeline_compare_run"
_COMPARE_LOCAL = "pipeline_comparison_local"
_COMPARE_DIGEST_FIELDS = (
    "source_digest",
    "order_digest",
    "configuration_digest",
    "bootstrap_corpus_digest",
    "holdout_corpus_digest",
)
# What `pipeline.py compare` gives the harness for a full run: the Interfaces
# section's variables without `LIMIT` and `SKEW`, and the timing file.
_COMPARE_VARIABLES = frozenset(
    {
        "TRACE_COMMONS_PG_TEST_DATABASE_URL",
        "TRACE_COMMONS_PIPELINE_COMPARE_BOOTSTRAP_PATH",
        "TRACE_COMMONS_PIPELINE_COMPARE_HOLDOUT_PATH",
        "TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH",
        "TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID",
        "TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH",
        "TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH",
        "TRACE_COMMONS_PIPELINE_COMPARE_TIMING_PATH",
        "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT",
        "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX",
        "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR",
        "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID",
        "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH",
    }
)
_COMPARE_CHECK_VARIABLES = frozenset(name for name in _COMPARE_VARIABLES if "_PIPELINE_CHECK_" in name)


def _comparison_side(compared=10, **overrides):
    """The distribution of one side for `compared` traces, each admitted and
    scored, with evidence of each gate branch when `compared` is 2 or more.
    The admission counts add up to `compared`, and each gate branch pair to
    `scored`, as `SideDistribution::observe` counts them."""
    side = {
        "admit": compared,
        "quarantine": 0,
        "reject": 0,
        "refused": 0,
        "other": 0,
        "scored": compared,
        "quality_passed": compared - compared * 4 // 10,
        "quality_failed": compared * 4 // 10,
        "novelty_passed": compared - compared // 2,
        "novelty_failed": compared // 2,
        "member": compared - compared // 2,
        "not_member": compared // 2,
        "chunks_capped": 0,
    }
    side.update(overrides)
    return side


_PERMITTED_RULE = "medium_risk_privacy_review"
_PERMITTED_RULES = [{"rule": _PERMITTED_RULE, "source": "ruling.PC-D22", "fields": ["admission"]}]
# `ComparisonRule::ALL` of `versioned_pipeline_comparison.rs`, in order.
_EXCLUDED_RULES = [
    {
        "rule": "deterministic_index_keys",
        "source": "compatibility_mapping.required_behavior_change_2",
        "fields": ["index_entry_id", "nearest_neighbor_hash"],
    },
    {"rule": "ledger_reason_text", "source": "ruling.T15-2", "fields": ["ledger_reason"]},
    {
        "rule": "shadow_values_not_in_contract",
        "source": "compatibility_mapping.membership_and_credit_rules",
        "fields": ["dedup_penalty", "contributor_cap", "anomaly_withheld"],
    },
]


def _compared_fields(compared=10, unexplained=None, trace_count=10, permitted=None, permitted_total=None):
    """The count fields of a report that compared `compared` of its
    `trace_count` traces. `unexplained` maps a compared field to its count;
    the pairs that differ are the last pairs. `permitted` maps a rule id to
    its count; those pairs come before the unexplained pairs.
    `permitted_total` is the number of permitted pairs (default: the sum of
    the counts, which is right while one rule permits a pair)."""
    unexplained = dict(unexplained or {})
    permitted = dict(permitted or {})
    if permitted_total is None:
        permitted_total = sum(permitted.values())
    total = max(unexplained.values(), default=0)
    return {
        "compared_count": compared,
        "partial": compared < trace_count,
        "equal_count": compared - total - permitted_total,
        "permitted_counts": permitted,
        "permitted_total": permitted_total,
        "unexplained_counts": unexplained,
        "unexplained_total": total,
        "unexplained": [
            {"position": position, "trace_hash": _fake_hash(f"trace-{position}"), "fields": sorted(unexplained)}
            for position in range(compared - total, compared)
        ],
        "first_unexplained_position": compared - total if total else None,
    }


def _comparison_report(check_id, **overrides):
    """A valid `trace_commons.pipeline_comparison_report.v1` report: a full
    run of 10 traces, each equal. `overrides` replaces fields before the
    report is signed."""
    report = {
        "schema": comparison.REPORT_SCHEMA,
        "check_id": check_id,
        "scope": "local_test",
        "production_ready": False,
        "external_payout_enabled": False,
        "partial": False,
        "skew": None,
        "safe_blockers": list(comparison.BLOCKERS),
        "pin": {field: _fake_hash(field) for field in _COMPARE_DIGEST_FIELDS},
        "bundle_id": _fake_hash("bundle"),
        **_candidate_digests(),
        "floors": {
            "perplexity_floor_micros": 2_500_000,
            "tail_fraction_floor_micros": 250_000,
            "novelty_floor_micros": 120_000,
        },
        "trace_count": 10,
        **_compared_fields(),
        "alignment_lost_position": None,
        "excluded_rules": json.loads(json.dumps(_EXCLUDED_RULES)),
        "permitted_rules": json.loads(json.dumps(_PERMITTED_RULES)),
        "branch_gaps": [],
        "records_digest": _fake_hash("records"),
    }
    report.update(overrides)
    # Each side observed each compared pair.
    compared = report.get("compared_count")
    report.setdefault(
        "distribution", {"baseline": _comparison_side(compared), "candidate": _comparison_side(compared)}
    )
    return _resigned(report)


def _write_compare_outputs(
    env,
    *,
    unexplained=None,
    permitted=None,
    branch_gaps=(),
    emit=True,
    compared=None,
    unexplained_list=None,
    result_overrides=None,
    evidence_overrides=None,
    **overrides,
):
    """What `pipeline_compare_run` leaves behind: the records file, the
    report, and, for a full run, one check result and its evidence (the
    emitter does nothing without its three variables). The pin digests and
    the trace count are those of the export manifest. `unexplained` maps a
    compared field to its count and `permitted` maps a rule id to its count. `compared` is the number of pairs before an
    early stop (default: the limit, or each trace). `overrides` replaces
    report fields, and `unexplained_list` the report's `unexplained` list.
    Returns the report."""
    records = b'{"position":0,"side":"baseline"}\n{"position":0,"side":"candidate"}\n'
    Path(env["TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH"]).write_bytes(records)
    manifest = json.loads(Path(env["TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH"]).read_text())
    trace_count = manifest["sample_count"]
    if compared is None:
        compared = min(int(env.get("TRACE_COMMONS_PIPELINE_COMPARE_LIMIT", trace_count)), trace_count)
    check_id = env["TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID"]
    fields = {
        "pin": {field: manifest[field] for field in _COMPARE_DIGEST_FIELDS},
        "trace_count": trace_count,
        **_compared_fields(compared, unexplained, trace_count, permitted),
        "skew": env.get("TRACE_COMMONS_PIPELINE_COMPARE_SKEW"),
        "branch_gaps": list(branch_gaps),
        "records_digest": _digest(records),
        **overrides,
    }
    if unexplained_list is not None:
        fields["unexplained"] = unexplained_list
    report = _comparison_report(check_id, **fields)
    report_bytes = results.canonical(report) + b"\n"
    Path(env["TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH"]).write_bytes(report_bytes)
    if not emit or report["partial"] or "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR" not in env:
        return report
    evidence = {
        "traces": report["compared_count"],
        "equal": report["equal_count"],
        "permitted": report["permitted_total"],
        "unexplained": 0,
        "records_hash": _digest(records),
        "report_hash": _digest(report_bytes),
    }
    evidence.update(evidence_overrides or {})
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
    raw.update(result_overrides or {})
    result_dir = Path(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"])
    (result_dir / f"{check_id}.result.json").write_text(json.dumps(raw))
    (result_dir / f"{check_id}.evidence.json").write_text(json.dumps(evidence))
    return report


class ComparisonReportValidationTests(unittest.TestCase):
    def _refused(self, report, label=None):
        with self.assertRaises(errors.ToolingError) as ctx:
            comparison.validate_comparison_report(report)
        if label is not None:
            self.assertEqual(str(ctx.exception), label)

    def test_a_valid_report_passes(self):
        for check_id in sorted(comparison.CHECK_IDS):
            with self.subTest(check_id):
                comparison.validate_comparison_report(_comparison_report(check_id))
        # A report of a failed run is valid: it names the differences.
        failed = _comparison_report(
            _COMPARE_LOCAL,
            **_compared_fields(5, {"admission": 2, "member": 1}),
            alignment_lost_position=4,
            skew="baseline_quality_floor",
            branch_gaps=[],
            distribution={"baseline": _comparison_side(5), "candidate": _comparison_side(5, admit=4, refused=1)},
        )
        comparison.validate_comparison_report(failed)
        # A report with permitted pairs is valid, and its Markdown view names
        # the rule with its source.
        with_permitted = _comparison_report(
            _COMPARE_LOCAL, **_compared_fields(10, {"admission": 1}, permitted={_PERMITTED_RULE: 2})
        )
        comparison.validate_comparison_report(with_permitted)
        text = comparison.markdown(with_permitted)
        self.assertIn("| Permitted | 2 |", text)
        self.assertIn(f"- `{_PERMITTED_RULE}` (ruling.PC-D22): 2", text)
        self.assertIn(f"- `{_PERMITTED_RULE}` (ruling.PC-D22): 0", comparison.markdown(_comparison_report(_COMPARE_LOCAL)))
        # The list of unexplained traces stops at 1,000 entries.
        long_list = _comparison_report(
            _COMPARE_LOCAL,
            trace_count=2000,
            compared_count=2000,
            equal_count=999,
            unexplained_counts={"admission": 1001},
            unexplained_total=1001,
            unexplained=[
                {"position": position, "trace_hash": _fake_hash(f"trace-{position}"), "fields": ["admission"]}
                for position in range(comparison.UNEXPLAINED_LIST_LIMIT)
            ],
            first_unexplained_position=0,
        )
        comparison.validate_comparison_report(long_list)
        # Each compared field name, `record_pair`, and each gap label is valid.
        every_field = {field: 1 for field in (*_COMPARED_FIELD_NAMES, "record_pair")}
        self.assertEqual(len(every_field), 23)
        comparison.validate_comparison_report(_comparison_report(_COMPARE_LOCAL, **_compared_fields(10, every_field)))
        comparison.validate_comparison_report(_comparison_report(_COMPARE_LOCAL, branch_gaps=list(_BRANCH_GAP_LABELS)))

        text = comparison.markdown(failed)
        self.assertTrue(text.startswith("# Pipeline comparison report\n"))
        for expected in (
            f"`{_COMPARE_LOCAL}`",
            failed["bundle_id"],
            failed["pin"]["configuration_digest"],
            "perplexity_floor_micros",
            "| refused | 0 | 1 |",
            "`admission`: 2",
            failed["unexplained"][0]["trace_hash"],
            "baseline_derived_scan_removed",
            "position 4",
            "Partial run: yes",
            "baseline_quality_floor",
        ):
            self.assertIn(expected, text)
        passed = comparison.markdown(_comparison_report(_COMPARE_LOCAL))
        self.assertIn("Partial run: no", passed)
        self.assertNotIn("stopped at position", passed)
        # Only the first 20 unexplained traces are in the Markdown view.
        self.assertIn(long_list["unexplained"][19]["trace_hash"], comparison.markdown(long_list))
        self.assertNotIn(long_list["unexplained"][20]["trace_hash"], comparison.markdown(long_list))

    def test_comparison_report_refuses_private_content(self):
        entry = {"position": 9, "trace_hash": _fake_hash("trace-9"), "fields": ["admission"]}
        with_entry = dict(_compared_fields(10, {"admission": 1}))
        cases = {
            "private_report_field": _comparison_report(_COMPARE_LOCAL, input="the text of one trace"),
            "private_report_field:nested": _comparison_report(
                _COMPARE_LOCAL, **{**with_entry, "unexplained": [{**entry, "secret_probe": "probe"}]}
            ),
            # A raw UUID is not a hash: the report must hold `trace_hash` only.
            "unsafe_report": _comparison_report(
                _COMPARE_LOCAL, **{**with_entry, "unexplained": [{**entry, "trace_hash": str(uuid.uuid4())}]}
            ),
            "unsafe_report_value": _comparison_report(_COMPARE_LOCAL, skew="token-compare-baseline-gate"),
            "unsafe_report_value:text": _comparison_report(_COMPARE_LOCAL, branch_gaps=["free text with spaces"]),
            "unsafe_report_value:float": _comparison_report(
                _COMPARE_LOCAL,
                floors={"perplexity_floor_micros": 2.5, "tail_fraction_floor_micros": 1, "novelty_floor_micros": 1},
            ),
        }
        for case, report in cases.items():
            with self.subTest(case=case):
                self._refused(report, case.split(":")[0])

    def test_counts_must_agree(self):
        two = _compared_fields(10, {"admission": 2})
        cases = (
            ("comparison_count_mismatch", {"equal_count": 9}),
            ("comparison_count_mismatch", {"permitted_counts": {_PERMITTED_RULE: 1}, "permitted_total": 1}),
            ("comparison_count_mismatch", {**two, "equal_count": 9}),
            # The list holds each unexplained trace, up to 1,000.
            ("comparison_count_mismatch", {**two, "unexplained": two["unexplained"][:1]}),
            ("comparison_count_mismatch", {"unexplained": two["unexplained"]}),
            ("comparison_count_mismatch", {"compared_count": 11, "equal_count": 11}),
            ("comparison_partial_mismatch", {"partial": True}),
            ("comparison_partial_mismatch", {**_compared_fields(3), "partial": False}),
            ("comparison_report_malformed", {"alignment_lost_position": -1}),
            ("comparison_report_malformed", {"alignment_lost_position": "4"}),
            ("comparison_report_malformed", {"alignment_lost_position": True}),
            ("comparison_report_malformed", {"alignment_lost_position": [4]}),
        )
        for label, overrides in cases:
            with self.subTest(label=label, overrides=sorted(overrides)):
                self._refused(_comparison_report(_COMPARE_LOCAL, **overrides), label)
        # A permitted pair is in the sum.
        comparison.validate_comparison_report(
            _comparison_report(_COMPARE_LOCAL, equal_count=9, permitted_counts={_PERMITTED_RULE: 1}, permitted_total=1)
        )

    def test_a_pair_that_two_rules_permit_is_one_pair(self):
        second = {"rule": "second_rule", "source": "ruling.PC-D99", "fields": ["member"]}
        with mock.patch.object(comparison, "PERMITTED_RULES", (*comparison.PERMITTED_RULES, second)):
            both = {_PERMITTED_RULE: 1, "second_rule": 1}
            fields = {"permitted_rules": [*_PERMITTED_RULES, second]}
            # One pair, counted for each of its two rules.
            report = _comparison_report(
                _COMPARE_LOCAL, **fields, **_compared_fields(10, permitted=both, permitted_total=1)
            )
            comparison.validate_comparison_report(report)
            self.assertIn("| Permitted | 1 |", comparison.markdown(report))
            # The total does not agree with the equal and unexplained counts.
            wrong = _comparison_report(
                _COMPARE_LOCAL, **fields, **{**_compared_fields(10, permitted=both, permitted_total=1), "equal_count": 8}
            )
            self._refused(wrong, "comparison_count_mismatch")
            # The sum of the rule counts is not the count of pairs.
            summed = _comparison_report(
                _COMPARE_LOCAL, **fields, **{**_compared_fields(10, permitted=both), "equal_count": 8}
            )
            comparison.validate_comparison_report(summed)
            more = _comparison_report(
                _COMPARE_LOCAL, **fields, **_compared_fields(10, permitted={_PERMITTED_RULE: 1}, permitted_total=2)
            )
            self._refused(more, "comparison_report_malformed")
            # A rule permits at most each permitted pair: one pair cannot give
            # a rule the count 2.
            fewer = _comparison_report(
                _COMPARE_LOCAL, **fields, **_compared_fields(10, permitted={_PERMITTED_RULE: 2, "second_rule": 1}, permitted_total=1)
            )
            self._refused(fewer, "comparison_report_malformed")

    def test_the_digest_is_checked(self):
        report = _comparison_report(_COMPARE_LOCAL)
        self._refused({**report, "equal_count": 9, "permitted_counts": {_PERMITTED_RULE: 1}, "permitted_total": 1}, "report_digest_mismatch")
        self._refused({**report, "report_digest": _fake_hash("another")}, "report_digest_mismatch")

    def test_the_blockers_and_the_scope_are_required(self):
        def without_blocker(name):
            return [blocker for blocker in comparison.BLOCKERS if blocker != name]

        self.assertEqual(
            comparison.BLOCKERS,
            (
                *corpus.LOCAL_BLOCKERS,
                "deterministic_privacy_only",
                "baseline_derived_scan_removed",
                "duplicate_short_circuits_not_compared",
                "review_start_privacy_pass_not_compared",
            ),
        )
        cases = (
            ("missing_local_blockers", {"safe_blockers": without_blocker("deterministic_privacy_only")}),
            ("missing_local_blockers", {"safe_blockers": without_blocker("baseline_derived_scan_removed")}),
            # PC-D24 and PC-D25: a report that does not name the two paths
            # of `main` that the run does not compare is not valid.
            ("missing_local_blockers", {"safe_blockers": without_blocker("duplicate_short_circuits_not_compared")}),
            ("missing_local_blockers", {"safe_blockers": without_blocker("review_start_privacy_pass_not_compared")}),
            ("missing_local_blockers", {"safe_blockers": without_blocker("local_test_only")}),
            ("invalid_report_scope", {"production_ready": True}),
            ("invalid_report_scope", {"scope": "production"}),
            ("payout_enabled", {"external_payout_enabled": True}),
            ("invalid_report_check_id", {"check_id": "pipeline_http_corpus_hf_local"}),
            ("unsupported_report_schema", {"schema": corpus.REPORT_SCHEMA}),
        )
        for label, overrides in cases:
            with self.subTest(label=label, overrides=overrides):
                check_id = overrides.pop("check_id", _COMPARE_LOCAL)
                self._refused(_comparison_report(check_id, **overrides), label)

    def test_the_comparison_blockers_agree_with_the_harness(self):
        source = (environment.ROOT / "crates/trace-commons-server/src/versioned_pipeline_comparison.rs").read_text()
        match = re.search(r"const COMPARISON_BLOCKERS: \[&str; \d+\] = \[(.*?)\];", source, re.S)
        self.assertIsNotNone(match)
        self.assertEqual(tuple(re.findall(r'"([a-z0-9_]+)"', match.group(1))), comparison.BLOCKERS)

    def test_the_excluded_rules_are_the_closed_list(self):
        self.assertEqual(list(comparison.EXCLUDED_RULES), _EXCLUDED_RULES)
        comparison.validate_comparison_report(_comparison_report(_COMPARE_LOCAL))
        first, second, third = _EXCLUDED_RULES
        for excluded in (
            [],
            [first],
            [first, second],
            [second, first, third],
            [*_EXCLUDED_RULES, {"rule": "another_rule", "source": "ruling.PC-D99", "fields": ["member"]}],
            [{**first, "fields": ["index_entry_id"]}, second, third],
            [{**first, "source": "ruling.PC-D99"}, second, third],
            [first, second, {**third, "fields": [*third["fields"], "admission"]}],
        ):
            with self.subTest(excluded=[rule.get("rule") for rule in excluded]):
                self._refused(_comparison_report(_COMPARE_LOCAL, excluded_rules=excluded), "comparison_report_malformed")

    def test_each_side_counts_each_compared_pair(self):
        # A partial run and an empty run are valid when the counts agree.
        comparison.validate_comparison_report(_comparison_report(_COMPARE_LOCAL, **_compared_fields(3)))
        comparison.validate_comparison_report(_comparison_report(_COMPARE_LOCAL, **_compared_fields(0)))
        cases = {
            # The admission counts of a side add up to `compared_count`.
            "admission sum low": _comparison_side(admit=9),
            "admission sum high": _comparison_side(quarantine=1),
            "refused beside each admit": _comparison_side(refused=1),
            # `scored` is the sum of each gate branch pair.
            "scored above quality": _comparison_side(quality_passed=5),
            "scored below quality": _comparison_side(quality_failed=5),
            "novelty": _comparison_side(novelty_failed=4),
            "member": _comparison_side(not_member=6),
            "scored alone": _comparison_side(scored=9),
            # A capped trace is a scored trace.
            "capped": _comparison_side(chunks_capped=11, admit=10),
        }
        for case, side in cases.items():
            for name in ("baseline", "candidate"):
                with self.subTest(case=case, side=name):
                    sides = {"baseline": _comparison_side(), "candidate": _comparison_side()}
                    sides[name] = side
                    self._refused(_comparison_report(_COMPARE_LOCAL, distribution=sides), "comparison_count_mismatch")
        # The distribution of a partial run counts its compared pairs, not
        # each trace of the pin.
        self._refused(
            _comparison_report(
                _COMPARE_LOCAL,
                **_compared_fields(3),
                distribution={"baseline": _comparison_side(), "candidate": _comparison_side()},
            ),
            "comparison_count_mismatch",
        )
        # A quarantined trace that the review approved is scored: `scored`
        # can exceed `admit`.
        quarantined = _comparison_side(admit=4, quarantine=6)
        comparison.validate_comparison_report(
            _comparison_report(_COMPARE_LOCAL, distribution={"baseline": quarantined, "candidate": quarantined})
        )

    def test_a_malformed_report_gives_a_label(self):
        base = _comparison_report(_COMPARE_LOCAL)
        self.assertEqual(len(base), 30)
        for field in base:
            with self.subTest(missing=field):
                report = {key: value for key, value in base.items() if key != field}
                if field != "report_digest":
                    report = _resigned(report)
                self._refused(report, "unsupported_report_schema" if field == "schema" else "comparison_report_malformed")

        def without(section, field):
            value = json.loads(json.dumps(base[section]))
            target = value["candidate"] if section == "distribution" else value
            del target[field]
            return {section: value}

        one = _compared_fields(10, {"admission": 1})
        malformed = (
            without("distribution", "refused"),
            without("floors", "novelty_floor_micros"),
            without("pin", "configuration_digest"),
            {"distribution": {"baseline": _comparison_side()}},
            {"distribution": {"baseline": _comparison_side(), "candidate": _comparison_side(refused=-1)}},
            {"unexplained_counts": ["admission"]},
            {"permitted_counts": None},
            {"branch_gaps": "novelty_passed_false"},
            {"unexplained": None},
            {**_compared_fields(10, {"admission": 1}), "unexplained": [None]},
            {**_compared_fields(10, {"admission": 1}), "unexplained": [{"position": 9}]},
            {"excluded_rules": [None]},
            # The list of permitted rules is the closed list, and the counts
            # name a rule of it.
            {"permitted_rules": []},
            {"permitted_rules": None},
            {"permitted_rules": [{**_PERMITTED_RULES[0], "rule": "another_rule"}]},
            {"permitted_rules": [{**_PERMITTED_RULES[0], "source": "ruling.PC-D99"}]},
            {"permitted_rules": [{**_PERMITTED_RULES[0], "fields": ["admission", "member"]}]},
            {"permitted_rules": [*_PERMITTED_RULES, {"rule": "another_rule", "source": "ruling.PC-D99", "fields": ["member"]}]},
            {"permitted_rules": [{**_PERMITTED_RULES[0], "extra": 1}]},
            _compared_fields(10, permitted={"another_rule": 1}),
            _compared_fields(10, permitted={"deterministic_index_keys": 1}),
            # The total counts pairs: it is zero exactly when no rule permitted
            # a pair, and it is at most the sum of the rule counts.
            {"permitted_total": 1},
            {"permitted_counts": {_PERMITTED_RULE: 1}, "permitted_total": 0},
            {"permitted_counts": {_PERMITTED_RULE: 1}, "permitted_total": 2, "equal_count": 7},
            {"permitted_total": -1},
            {"permitted_total": "1"},
            {"skew": "another_skew"},
            {"partial": 0},
            {"trace_count": "10"},
            {"first_unexplained_position": -1},
            {"extra_field": 1},
            # Values that the harness can never write: a name outside the
            # closed sets, a position outside the compared pairs, and counts
            # that do not agree on "no unexplained trace".
            {"branch_gaps": ["A1B2C3D4-0000-4000-8000-ABCDEFABCDEF"]},
            {"branch_gaps": ["s04m.jsonl"]},
            {"branch_gaps": ["member_true", "admission"]},
            {**one, "unexplained_counts": {"tenant-a": 1}},
            {**one, "unexplained_counts": {"admission": 1, "position": 1}},
            {**one, "unexplained": [{**one["unexplained"][0], "fields": ["tenant_id"]}]},
            {**one, "unexplained": [{**one["unexplained"][0], "fields": ["admission", "s04m.jsonl"]}]},
            {"unexplained_counts": {"admission": 1}},
            {**one, "unexplained_counts": {}},
            {"first_unexplained_position": 7},
            {**one, "first_unexplained_position": None},
            {"alignment_lost_position": 99},
            {"alignment_lost_position": 10},
            {**_compared_fields(5), "alignment_lost_position": 5},
        )
        for overrides in malformed:
            with self.subTest(overrides=overrides):
                self._refused(_comparison_report(_COMPARE_LOCAL, **overrides), "comparison_report_malformed")
        self._refused(_comparison_report(_COMPARE_LOCAL, bundle_id="bundle"), "invalid_report_hash")
        self._refused(_comparison_report(_COMPARE_LOCAL, records_digest="records"), "invalid_report_hash")
        for value in ([], "report", 3, None):
            with self.subTest(value=value):
                self._refused(value)


# The compared field names of the Interfaces section, and the labels of the
# gate branches with no evidence.
_COMPARED_FIELD_NAMES = (
    "receipt_code",
    "terminal",
    "privacy_risk",
    "privacy_basis",
    "admission",
    "scored",
    "quality_passed",
    "novelty_passed",
    "perplexity_micros",
    "tail_fraction_micros",
    "peak_perplexity_micros",
    "novelty_score_micros",
    "peak_novelty_micros",
    "chunk_count",
    "total_chunk_count",
    "chunks_capped",
    "index_cardinality",
    "credit_quality_micros",
    "credit_quality_version",
    "member",
    "member_chunks",
    "credit_events",
)
_BRANCH_GAP_LABELS = (
    "quality_passed_true",
    "quality_passed_false",
    "novelty_passed_true",
    "novelty_passed_false",
    "member_true",
    "member_false",
)


def _compare_pin(**overrides):
    """A comparison pin with its five digests. A value of `_DROP` removes
    the field."""
    pin = {
        "schema": corpus.PIN_SCHEMA,
        "repository": "jedisct1/security-audits",
        "revision": "deadbeef",
        "split": "train",
        "translator": "swival",
        "bootstrap_count": 4,
        "holdout_count": 6,
        "min_words": 1,
        "max_words": 20000,
        "expected_instrument_count": 1,
        "with_events": True,
        **{field: _fake_hash(field) for field in _COMPARE_DIGEST_FIELDS},
        "local_jsonl_dir": "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl",
        "session_names": ["s01.jsonl", "s02.jsonl"],
    }
    pin.update(overrides)
    return {key: value for key, value in pin.items() if value is not _DROP}


class CompareExportTests(unittest.TestCase):
    """`comparison.export_compare_corpus`, with `run_child` replaced."""

    def setUp(self):
        self.run = _scratch_run("compare-export")
        self.tmp = Path(tempfile.mkdtemp())
        self.commands = []

    def tearDown(self):
        shutil.rmtree(self.run.run_dir, ignore_errors=True)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _export(self, pin, *, manifest=None, manifest_text=None, **options):
        """`manifest` replaces fields of the manifest. `manifest_text` is
        written as the whole file; `_DROP` leaves no file."""
        pin_path = self.tmp / "pin.json"
        pin_path.write_text(json.dumps(pin))
        if manifest is None:
            manifest = {}
        written = {
            **{field: pin[field] for field in _COMPARE_DIGEST_FIELDS if field in pin},
            "source": {"repository": pin["repository"], "with_events": True},
            "sample_count": 10,
            "contains_raw_trace_text": False,
            **manifest,
        }

        def fake_run_child(run, step, command, env):
            self.commands.append((step, list(command)))
            output_dir = Path(command[command.index("--output-dir") + 1])
            output_dir.mkdir(parents=True, exist_ok=True)
            if manifest_text is not _DROP:
                (output_dir / "source-manifest.json").write_text(manifest_text or json.dumps(written))

        with mock.patch.object(comparison, "run_child", fake_run_child):
            return comparison.export_compare_corpus(self.run, pin_path, {}, **options)

    def test_the_export_command_has_the_pin_and_the_event_flags(self):
        pin = _compare_pin(
            session_names=["s01.jsonl", "s04m.jsonl", "s04h.jsonl"],
            declared_privacy_risk={"s04m.jsonl": "medium", "s04h.jsonl": "high"},
        )
        outputs = self._export(pin, name="risk", local_dir="fixtures/pipeline-compare-jsonl")

        output_dir = self.run.run_dir / "compare" / "risk"
        self.assertEqual(
            outputs,
            (
                output_dir / "bootstrap-compare.jsonl",
                output_dir / "holdout-compare.jsonl",
                output_dir / "source-manifest.json",
            ),
        )
        [(step, command)] = self.commands
        self.assertEqual(step, "compare_export_risk")
        separator = command.index("--")
        self.assertEqual(
            command[:separator],
            ["cargo", "run", "-q", "-p", "trace-commons-server", "--bin", "trace-commons-pipeline-corpus-export"],
        )
        arguments = command[separator + 1 :]

        def values(flag):
            return [arguments[index + 1] for index, item in enumerate(arguments) if item == flag]

        self.assertIn("--with-events", arguments)
        self.assertEqual(values("--output-dir"), [str(output_dir)])
        self.assertEqual(values("--session-name"), ["s01.jsonl", "s04m.jsonl", "s04h.jsonl"])
        self.assertEqual(values("--declared-privacy-risk"), ["s04m.jsonl=medium", "s04h.jsonl=high"])
        self.assertEqual(values("--local-jsonl-dir"), ["fixtures/pipeline-compare-jsonl"])
        self.assertEqual(values("--cache-dir"), [str(corpus.HF_CACHE_DIR)])
        for flag, field in (
            ("--repository", "repository"),
            ("--revision", "revision"),
            ("--split", "split"),
            ("--translator", "translator"),
            ("--bootstrap-count", "bootstrap_count"),
            ("--holdout-count", "holdout_count"),
            ("--min-words", "min_words"),
            ("--max-words", "max_words"),
            ("--expected-instrument-count", "expected_instrument_count"),
            ("--expected-source-digest", "source_digest"),
            ("--expected-order-digest", "order_digest"),
        ):
            self.assertEqual(values(flag), [str(pin[field])], flag)

    def test_release_selects_the_optimized_build(self):
        pin = _compare_pin()
        self._export(pin, name="local", local_dir="fixtures")
        self._export(pin, name="corpus", local_dir="fixtures", release=True)
        (_, debug), (step, optimized) = self.commands
        self.assertNotIn("--release", debug)
        self.assertEqual(step, "compare_export_corpus")
        self.assertIn("--release", optimized)
        self.assertLess(optimized.index("--release"), optimized.index("--"))
        self.assertEqual([item for item in optimized if item != "--release"][:7], debug[:7])

    def test_the_export_refuses_a_pin_or_a_manifest_that_does_not_agree(self):
        # The two list fields select files of a local directory.
        network = {"local_jsonl_dir": _DROP}
        for fields in (
            {"session_names": ["s01.jsonl"]},
            {"session_names": _DROP, "declared_privacy_risk": {"s01.jsonl": "high"}},
        ):
            with self.subTest(fields=sorted(fields)):
                with self.assertRaises(errors.ToolingError) as ctx:
                    self._export(_compare_pin(**network, **fields), name="corpus")
                self.assertEqual(str(ctx.exception), "comparison_pin_field_needs_local_dir")
                self.assertEqual(self.commands, [], "refused before the export starts")
        # A network pin without them is exported with no local directory.
        self._export(_compare_pin(**network, session_names=_DROP), name="corpus")
        [(_, command)] = self.commands
        self.assertNotIn("--local-jsonl-dir", command)
        self.assertNotIn("--session-name", command)
        self.assertNotIn("--declared-privacy-risk", command)

        for field in _COMPARE_DIGEST_FIELDS:
            with self.subTest(mismatch=field):
                with self.assertRaises(errors.ToolingError) as ctx:
                    self._export(_compare_pin(), name="corpus", manifest={field: _fake_hash("another")})
                self.assertEqual(str(ctx.exception), f"hf_{field}_mismatch")
        # A pin that does not have a digest yet does not compare it.
        self._export(
            _compare_pin(configuration_digest=_DROP),
            name="corpus",
            manifest={"configuration_digest": _fake_hash("another")},
        )
        for label, manifest in (
            ("comparison_manifest_without_events", {"source": {"repository": "jedisct1/security-audits"}}),
            ("comparison_manifest_without_events", {"source": None}),
            ("hf_manifest_contains_raw_trace_text", {"contains_raw_trace_text": True}),
        ):
            with self.subTest(label=label):
                with self.assertRaises(errors.ToolingError) as ctx:
                    self._export(_compare_pin(), name="corpus", manifest=manifest)
                self.assertEqual(str(ctx.exception), label)

    def test_an_export_without_a_manifest_object_gives_a_label(self):
        for index, manifest_text in enumerate((_DROP, "{not json", "[]", '"manifest"', "null")):
            with self.subTest(manifest_text=manifest_text):
                with self.assertRaises(errors.ToolingError) as ctx:
                    self._export(_compare_pin(), name=f"corpus{index}", manifest_text=manifest_text)
                self.assertEqual(str(ctx.exception), "comparison_manifest_malformed")


class CompareCommandTests(_CorpusRunCase):
    _SELF_TEST = {
        "compare_self_pass": {},
        "compare_self_risk": {"fail": True, "unexplained": {"admission": 1}, "permitted": {_PERMITTED_RULE: 1}},
        # The skew stops the run at its first pair (PC-D20).
        "compare_self_skew": {
            "fail": True,
            "compared": 1,
            "unexplained": {"quality_passed": 1, "member": 1},
            "alignment_lost_position": 0,
        },
        "compare_self_repeat": {},
    }

    def setUp(self):
        super().setUp()
        self.local_pin = str(pipeline.COMPARE_LOCAL_PIN)
        self.local = self.tmp / "local"
        # What the fake export writes: `sample_count` (default: the two
        # counts of the pin), and the whole manifest file when a test sets
        # `manifest_text` (`_DROP` leaves no file).
        self.sample_count = None
        self.manifest_text = None

    def _fake_export(self, run, pin_path, env, *, name, local_dir=None, release=False):
        """The export: the two corpus files and a manifest with the digests
        of the pin (a digest that the pin lacks gets a value)."""
        self.calls.append(("export", Path(pin_path), name, local_dir, release))
        pin = json.loads(Path(pin_path).read_text())
        sample_count = pin["bootstrap_count"] + pin["holdout_count"]
        manifest = {
            **{field: pin.get(field, _fake_hash(f"export-{field}")) for field in _COMPARE_DIGEST_FIELDS},
            "source": {"with_events": True},
            "sample_count": sample_count if self.sample_count is None else self.sample_count,
            "contains_raw_trace_text": False,
        }
        output_dir = run.run_dir / "compare" / name
        output_dir.mkdir(parents=True, exist_ok=True)
        paths = tuple(
            output_dir / file for file in ("bootstrap-compare.jsonl", "holdout-compare.jsonl", "source-manifest.json")
        )
        for path in paths[:2]:
            path.write_text("{}\n")
        if self.manifest_text is not _DROP:
            paths[2].write_text(self.manifest_text or json.dumps(manifest))
        return paths

    def _harness(self, default=None, **by_step):
        """`cargo_test` replaced by the comparison harness. The entry of
        `by_step` for the step (or `default`) holds the arguments of
        `_write_compare_outputs`, and two more: `write=False` for a harness
        that leaves no file, and `fail=True` for one that fails after it
        wrote its files (and emits no check result, as the harness does)."""

        def fake_cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            self.calls.append(("cargo", step, tuple(cargo_args), test_filter, dict(env), exact, ignored))
            behavior = dict(by_step.get(step, default or {}))
            fail = behavior.pop("fail", False)
            if behavior.pop("write", True):
                _write_compare_outputs(env, **{"emit": not fail, **behavior})
            if fail:
                raise errors.StepFailed(step, 101, run.log_path(step))

        return fake_cargo_test

    def _compare(self, argv, harness=None):
        self.stdout, self.stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(pipeline, "export_compare_corpus", self._fake_export):
            return self._main(
                ["compare", *argv, "--postgres-admin-url", _ADMIN_URL], cargo=harness or self._harness()
            )

    def _corpus(self, harness=None, *extra, pin=None):
        return self._compare(["--corpus", pin or self.local_pin, *extra], harness)

    def _again(self):
        """A new run directory: one run has one result for a check id."""
        shutil.rmtree(self.run.run_dir)
        self.run = _scratch_run("corpus")
        self.calls.clear()

    def _write_pin(self, **overrides):
        path = self.tmp / "pin.json"
        path.write_text(json.dumps(_compare_pin(**overrides)))
        return str(path)

    def _failure(self):
        return self.stderr.getvalue().strip()

    def _exports(self):
        return [call for call in self.calls if call[0] == "export"]

    def _report_path(self, check_id=_COMPARE_LOCAL, partial=False, failed=False):
        suffix = "-partial" if partial else "-failed" if failed else ""
        return self.local / f"pipeline-comparison-{check_id}{suffix}.json"

    def _local_files(self):
        """The names of the files under `.local/`."""
        return sorted(path.name for path in self.local.glob("*")) if self.local.is_dir() else []

    def test_compare_passes_only_the_expected_variables(self):
        code = self._corpus()

        self.assertEqual(code, 0, self.stderr.getvalue())
        [(_, pin_path, name, local_dir, _)] = self._exports()
        self.assertEqual(pin_path, pipeline.COMPARE_LOCAL_PIN)
        self.assertEqual(
            Path(local_dir), environment.ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl"
        )
        kinds = [call[0] for call in self.calls]
        self.assertLess(kinds.index("export"), kinds.index("invoke"), "the export is checked before a database starts")
        [(_, step, _, test_filter, env, exact, ignored)] = self._cargo_calls()
        self.assertEqual(step, "compare_run")
        self.assertEqual(test_filter, _COMPARE_HARNESS)
        self.assertEqual(pipeline.COMPARE_HARNESS, _COMPARE_HARNESS)
        self.assertTrue(exact)
        self.assertTrue(ignored)
        self.assertEqual({key for key in env if key.startswith("TRACE_COMMONS_")}, _COMPARE_VARIABLES)
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID"], _COMPARE_LOCAL)
        self.assertIn("/admission_test_", env["TRACE_COMMONS_PG_TEST_DATABASE_URL"])
        export_dir = self.run.run_dir / "compare" / name
        for variable, path in (
            ("BOOTSTRAP_PATH", export_dir / "bootstrap-compare.jsonl"),
            ("HOLDOUT_PATH", export_dir / "holdout-compare.jsonl"),
            ("MANIFEST_PATH", export_dir / "source-manifest.json"),
            # Each file of a harness run is in the run directory, never
            # directly under `.local/`: the harness removes a report that
            # exists at its report path.
            ("REPORT_PATH", self.run.run_dir / "compare_run-report.json"),
            ("RECORDS_PATH", self.run.run_dir / "compare_run-records.jsonl"),
            ("TIMING_PATH", self.run.run_dir / "compare_run-timing.jsonl"),
        ):
            self.assertEqual(env[f"TRACE_COMMONS_PIPELINE_COMPARE_{variable}"], str(path), variable)
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"], str(self.run.results_dir))
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"], self.run.run_id)
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"], self.run.code_revision_hash)
        master_key = env["TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX"]
        self.assertRegex(master_key, r"^[0-9a-f]{64}$")
        self.assertNotIn(master_key, self.stdout.getvalue() + self.stderr.getvalue())

        lines = [line for line in self.stdout.getvalue().splitlines() if line]
        self.assertEqual(len(lines), 1, lines)
        self.assertRegex(
            lines[0],
            r"^PipelineCompareOK: traces=10 equal=10 permitted=0 unexplained=0 partial=false "
            rf"seconds=[0-9]+ run={self.run.run_id} report=pipeline-comparison-{_COMPARE_LOCAL}\.json$",
        )
        # The local report is a copy of the bytes that the evidence names.
        local_report = self._report_path()
        harness_bytes = (self.run.run_dir / "compare_run-report.json").read_bytes()
        self.assertEqual(local_report.read_bytes(), harness_bytes)
        evidence = json.loads((self.run.results_dir / f"{_COMPARE_LOCAL}.evidence.json").read_text())
        self.assertEqual(evidence["report_hash"], _digest(local_report.read_bytes()))
        self.assertTrue(local_report.with_suffix(".md").read_text().startswith("# Pipeline comparison report\n"))
        self.assertFalse((self.local / "pipeline-lab-catalog.json").exists(), "compare writes no catalog")
        self.assertEqual(self.tree_hash_calls, 1, "the tree is examined again at the end")

    def test_a_pin_without_events_is_refused(self):
        for overrides in ({"with_events": _DROP}, {"with_events": False}, {"with_events": "true"}):
            with self.subTest(overrides=overrides):
                self.assertEqual(self._corpus(pin=self._write_pin(**overrides)), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_without_events")
                self.assertEqual(self.calls, [], "refused before any export, cargo, or Docker call")
        # The declared risks and `trace_file_for` change only the corpus
        # files, so a full run needs each digest of the pin.
        optional = [field for field in _COMPARE_DIGEST_FIELDS if field not in ("source_digest", "order_digest")]
        for field in optional:
            with self.subTest(missing=field):
                self.assertEqual(self._corpus(pin=self._write_pin(**{field: _DROP})), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_digest_missing")
                self.assertEqual(self.calls, [], "refused before any export, cargo, or Docker call")
        for field in ("source_digest", "order_digest"):
            with self.subTest(missing=field):
                self.assertEqual(self._corpus(pin=self._write_pin(**{field: _DROP})), 1)
                self.assertEqual(self._failure(), "PipelineFailure: pin_missing_field")
                self.assertEqual(self.calls, [])
        # A partial run is not evidence: a pin that has no corpus digest yet
        # can have one.
        draft = self._write_pin(bootstrap_corpus_digest=_DROP, holdout_corpus_digest=_DROP)
        self.assertEqual(self._corpus(None, "--limit", "3", pin=draft), 0, self.stderr.getvalue())
        self.assertIn("partial=true", self.stdout.getvalue())
        # A limit that does not make the run partial does not remove the
        # rule. The manifest has the trace count, so the run is refused
        # before a database starts.
        for limit in ("10", "11"):
            with self.subTest(limit=limit):
                self._again()
                self.assertEqual(self._corpus(None, "--limit", limit, pin=draft), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_digest_missing")
                self.assertEqual([call[0] for call in self.calls], ["export"], "no database and no harness")
        # The same rule after a pass, from the report's own `partial`.
        self._again()
        self.assertEqual(self._corpus(self._harness({"compared": 10}), "--limit", "3", pin=draft), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_digest_missing")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        self.assertFalse(self._report_path().exists())

    def test_a_network_pin_uses_the_hf_check_id(self):
        pin = self._write_pin(local_jsonl_dir=_DROP, session_names=_DROP)
        self.assertEqual(self._corpus(pin=pin), 0, self.stderr.getvalue())
        [(_, _, _, local_dir, _)] = self._exports()
        self.assertIsNone(local_dir)
        [(_, _, _, _, env, _, _)] = self._cargo_calls()
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID"], "pipeline_comparison_hf")
        self.assertEqual(set(results.load_results(self.run)), {"pipeline_comparison_hf"})
        self.assertTrue(self._report_path("pipeline_comparison_hf").is_file())
        self.assertFalse(self._report_path().exists())
        self.assertIn("report=pipeline-comparison-pipeline_comparison_hf.json", self.stdout.getvalue())

    def test_limit_makes_a_partial_run(self):
        self.assertEqual(self._corpus(None, "--limit", "3"), 0, self.stderr.getvalue())
        [(_, _, _, _, env, _, _)] = self._cargo_calls()
        self.assertEqual(env["TRACE_COMMONS_PIPELINE_COMPARE_LIMIT"], "3")
        self.assertEqual(
            {key for key in env if key.startswith("TRACE_COMMONS_")},
            _COMPARE_VARIABLES | {"TRACE_COMMONS_PIPELINE_COMPARE_LIMIT"},
        )
        self.assertEqual(results.load_results(self.run), {}, "a partial run has no check result")
        self.assertRegex(
            self.stdout.getvalue(),
            r"^PipelineCompareOK: traces=3 equal=3 permitted=0 unexplained=0 partial=true seconds=[0-9]+ ",
        )
        # A partial run does not replace the report of the full run.
        self.assertTrue(self._report_path(partial=True).is_file())
        self.assertTrue(self._report_path(partial=True).with_suffix(".md").is_file())
        self.assertFalse(self._report_path().exists())
        self.assertIn("report=pipeline-comparison-pipeline_comparison_local-partial.json", self.stdout.getvalue())

    def test_unexplained_differences_fail_with_their_label(self):
        code = self._corpus(self._harness({"fail": True, "unexplained": {"admission": 2}}))

        self.assertNotEqual(code, 0)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_has_unexplained_differences")
        self.assertEqual(
            self.stdout.getvalue().splitlines(),
            [
                "PipelineCompareReport: unexplained=2 alignment_lost=none partial=false "
                f"run={self.run.run_id} report=pipeline-comparison-{_COMPARE_LOCAL}-failed.json"
            ],
        )
        # The plain name is the name of a pass only.
        self.assertFalse(self._report_path().exists())
        report = json.loads(self._report_path(failed=True).read_text())
        self.assertEqual(report["unexplained_total"], 2)
        self.assertEqual(report["unexplained_counts"], {"admission": 2})
        self.assertIn("`admission`: 2", self._report_path(failed=True).with_suffix(".md").read_text())
        self.assertEqual(results.load_results(self.run), {})
        # A driver error that ends the run after one difference: the label is
        # the report's, and the line says that the run did not complete.
        self._again()
        cut_short = {"fail": True, "compared": 5, "unexplained": {"admission": 1}}
        self.assertEqual(self._corpus(self._harness(cut_short)), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_has_unexplained_differences")
        self.assertEqual(
            self.stdout.getvalue().splitlines(),
            [
                "PipelineCompareReport: unexplained=1 alignment_lost=none partial=true "
                f"run={self.run.run_id} report=pipeline-comparison-{_COMPARE_LOCAL}-partial.json"
            ],
        )

    def test_a_branch_gap_fails_with_its_label(self):
        gap = {"branch_gaps": ["novelty_passed_false"]}
        self.assertEqual(self._corpus(self._harness({"fail": True, **gap})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_gate_branch_not_exercised")
        self.assertIn("PipelineCompareReport: unexplained=0 alignment_lost=none", self.stdout.getvalue())
        self.assertEqual(
            json.loads(self._report_path(failed=True).read_text())["branch_gaps"], ["novelty_passed_false"]
        )
        self.assertFalse(self._report_path().exists())
        # A harness that passes with a gap is refused with the same label.
        self._again()
        self.assertEqual(self._corpus(self._harness(gap)), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_gate_branch_not_exercised")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())

    def test_a_failed_step_without_a_report_keeps_the_step_failure(self):
        code = self._corpus(self._harness({"fail": True, "write": False}))
        self.assertEqual(code, 101)
        lines = [line for line in self.stderr.getvalue().splitlines() if line]
        self.assertEqual(len(lines), 1, lines)
        self.assertTrue(lines[0].startswith("PipelineFailure: step_failed:compare_run exit=101 log="), lines[0])
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertFalse(self._report_path().exists())

        # A report that is not valid does not replace the step failure.
        log_path = self.run.log_path("compare_run")
        for malformed in (b"{not json", b"[]", results.canonical(_comparison_report(_COMPARE_LOCAL, equal_count=3))):
            with self.subTest(malformed=malformed[:16]):

                def failing(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
                    Path(env["TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH"]).write_bytes(malformed)
                    raise errors.StepFailed(step, 101, log_path)

                self.assertEqual(self._corpus(failing), 101)
                self.assertTrue(self._failure().startswith("PipelineFailure: step_failed:compare_run exit=101 log="))
                self.assertEqual(self.stdout.getvalue(), "")
                self.assertFalse(self._report_path().exists())

    def test_a_pass_without_a_report_fails(self):
        self.assertEqual(self._corpus(self._harness({"write": False})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_report_missing")
        self.assertEqual(self.stdout.getvalue(), "")

    def test_a_report_that_is_not_json_fails(self):
        def harness(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            Path(env["TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH"]).write_bytes(b"{not json")

        self.assertEqual(self._corpus(harness), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_report_malformed")
        self.assertFalse(self._report_path().exists())

    def test_a_report_of_another_check_fails(self):
        """The check id comes from the pin: a report that names the other
        check id is not the report of this run."""

        for fail in (False, True):
            with self.subTest(fail=fail):
                self._again()

                def harness(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
                    other = {**env, "TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID": "pipeline_comparison_hf"}
                    _write_compare_outputs(other, emit=not fail)
                    if fail:
                        raise errors.StepFailed(step, 101, run.log_path(step))

                self.assertEqual(self._corpus(harness), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_report_check_mismatch")
                self.assertEqual(self.stdout.getvalue(), "")
                self.assertEqual(self._local_files(), [])

    def test_the_report_must_be_the_report_of_this_run(self):
        """The report's pin, trace count, and skew are those of the run, for
        a harness that passes and for one that fails."""
        pin = json.loads(pipeline.COMPARE_LOCAL_PIN.read_text())
        pinned = {field: pin[field] for field in _COMPARE_DIGEST_FIELDS}
        cases = [
            ((), {"pin": {field: _fake_hash(field) for field in _COMPARE_DIGEST_FIELDS}}, "comparison_report_pin_mismatch"),
            ((), {"trace_count": 12, "compared": 12}, "comparison_report_trace_count_mismatch"),
            # `--corpus` sets no skew: a report with one is not of this run.
            (("--limit", "3"), {"skew": "baseline_quality_floor"}, "comparison_report_skew_mismatch"),
            ((), {"compared": 3, "skew": "baseline_quality_floor"}, "comparison_report_skew_mismatch"),
        ]
        cases += [
            ((), {"pin": {**pinned, field: _fake_hash(f"another-{field}")}}, "comparison_report_pin_mismatch")
            for field in _COMPARE_DIGEST_FIELDS
        ]
        for extra, behavior, label in cases:
            for fail in (False, True):
                with self.subTest(label=label, behavior=sorted(behavior), fail=fail):
                    self._again()
                    self.assertEqual(self._corpus(self._harness({"fail": fail, **behavior}), *extra), 1)
                    self.assertEqual(self._failure(), f"PipelineFailure: {label}")
                    self.assertEqual(self.stdout.getvalue(), "")
                    self.assertEqual(self._local_files(), [])

    def test_a_pass_compares_the_expected_number_of_traces(self):
        cases = (
            # With no limit, a pass compares each trace of the pin.
            ((), {"compared": 3}),
            ((), {"compared": 0}),
            (("--limit", "3"), {"compared": 2}),
            (("--limit", "3"), {"compared": 0}),
            (("--limit", "3"), {"compared": 10}),
            (("--limit", "20"), {"compared": 3}),
        )
        for extra, behavior in cases:
            with self.subTest(extra=extra, behavior=behavior):
                self._again()
                self.assertEqual(self._corpus(self._harness(behavior), *extra), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_compared_count_mismatch")
                self.assertEqual(self.stdout.getvalue(), "")
                self.assertEqual(self._local_files(), [])
        # A limit above the trace count gives a full run.
        self._again()
        self.assertEqual(self._corpus(None, "--limit", "20"), 0, self.stderr.getvalue())
        self.assertIn("traces=10 equal=10 permitted=0 unexplained=0 partial=false", self.stdout.getvalue())
        self.assertEqual(set(results.load_results(self.run)), {_COMPARE_LOCAL})

    def test_the_results_of_a_run_are_exactly_its_own(self):
        def with_result(check_id):
            def harness(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
                _write_compare_outputs(env)
                _emit_check(env, check_id)

            return harness

        # A partial run has no check result.
        self.assertEqual(self._corpus(with_result(_COMPARE_LOCAL), "--limit", "3"), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_check_results_unexpected")
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertEqual(self._local_files(), [])
        # A full run has the result of its check and no other.
        self._again()
        self.assertEqual(self._corpus(with_result("pipeline_comparison_hf")), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_check_results_unexpected")
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertEqual(self._local_files(), [])

    def test_a_failed_step_with_a_report_of_a_pass_is_not_written_as_one(self):
        """A harness that fails after a full report with no failure (the app
        stop, the emitter): no check result exists for the report, so it
        does not get the name of the full-run report."""
        self.assertEqual(self._corpus(self._harness({"fail": True})), 101)
        self.assertTrue(self._failure().startswith("PipelineFailure: step_failed:compare_run exit=101 log="))
        name = f"pipeline-comparison-{_COMPARE_LOCAL}-failed"
        self.assertEqual(
            self.stdout.getvalue().splitlines(),
            [
                "PipelineCompareReport: unexplained=0 alignment_lost=none partial=false "
                f"run={self.run.run_id} report={name}.json"
            ],
        )
        self.assertEqual(self._local_files(), [f"{name}.json", f"{name}.md"])
        self.assertEqual(
            self._report_path(failed=True).read_bytes(), (self.run.run_dir / "compare_run-report.json").read_bytes()
        )
        # A partial report of a failed step keeps the name of a partial run.
        self._again()
        self.assertEqual(self._corpus(self._harness({"fail": True, "compared": 4})), 101)
        self.assertIn(f"partial=true run={self.run.run_id} report=", self.stdout.getvalue())
        self.assertTrue(self._report_path(partial=True).is_file())
        self.assertFalse(self._report_path().exists())

    def test_a_report_write_that_fails_keeps_the_failure_of_the_step(self):
        for behavior, code, start in (
            ({"fail": True}, 101, "PipelineFailure: step_failed:compare_run exit=101 log="),
            ({"fail": True, "unexplained": {"admission": 2}}, 1, "PipelineFailure: comparison_has_unexplained_differences"),
        ):
            with self.subTest(behavior=behavior):
                self._again()
                with mock.patch.object(pipeline, "atomic_write", side_effect=OSError("no space")):
                    self.assertEqual(self._corpus(self._harness(behavior)), code)
                lines = [line for line in self.stderr.getvalue().splitlines() if line]
                self.assertEqual(len(lines), 1, lines)
                self.assertTrue(lines[0].startswith(start), lines[0])
                self.assertEqual(self.stdout.getvalue(), "")

    def test_a_full_pass_without_its_records_file_gives_a_label(self):
        def harness(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            _write_compare_outputs(env)
            Path(env["TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH"]).unlink()

        self.assertEqual(self._corpus(harness), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_records_missing")
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertEqual(self._local_files(), [])

    def test_a_manifest_that_is_not_usable_gives_a_label(self):
        usable = {
            **{field: _fake_hash(field) for field in _COMPARE_DIGEST_FIELDS},
            "source": {"with_events": True},
            "sample_count": 10,
            "contains_raw_trace_text": False,
        }
        texts = [_DROP, "{not json", "[]"]
        texts += [json.dumps({key: value for key, value in usable.items() if key != field}) for field in ("sample_count", "order_digest")]
        texts += [json.dumps({**usable, **changed}) for changed in ({"sample_count": "10"}, {"sample_count": -1}, {"sample_count": True}, {"source_digest": "digest"})]
        for manifest_text in texts:
            for argv in (["--corpus", self.local_pin], ["--self-test"]):
                with self.subTest(manifest_text=manifest_text, argv=argv[0]):
                    self._again()
                    self.manifest_text = manifest_text
                    self.assertEqual(self._compare(argv), 1)
                    self.assertEqual(self._failure(), "PipelineFailure: comparison_manifest_malformed")
                    self.assertEqual({call[0] for call in self.calls}, {"export"}, "no database and no harness")

    def test_a_pin_file_that_cannot_be_read_gives_a_label(self):
        missing = self.tmp / "no-such-pin.json"
        not_json = self.tmp / "not-json.json"
        not_json.write_text("{not json")
        not_an_object = self.tmp / "list.json"
        not_an_object.write_text("[]")
        not_text = self.tmp / "bytes.json"
        not_text.write_bytes(b"\xff\xfe\x00")
        for pin_path in (missing, not_json, not_an_object, not_text, self.tmp):
            with self.subTest(pin=pin_path.name):
                self.assertEqual(self._corpus(pin=str(pin_path)), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_unreadable")
                self.assertEqual(self.calls, [], "refused before any export, cargo, or Docker call")
        # A JSON object that is not a pin keeps the label of `load_pin`.
        other = self.tmp / "other.json"
        other.write_text(json.dumps({"schema": corpus.CORPUS_SCHEMA}))
        self.assertEqual(self._corpus(pin=str(other)), 1)
        self.assertEqual(self._failure(), "PipelineFailure: unsupported_pin_schema")

    def test_a_pin_field_of_a_wrong_type_gives_a_label(self):
        cases = (
            {"local_jsonl_dir": ["x"]},
            {"local_jsonl_dir": 5},
            {"local_jsonl_dir": False},
            {"session_names": 5},
            {"session_names": "s01.jsonl"},
            {"session_names": ["s01.jsonl", 2]},
            {"declared_privacy_risk": ["s01.jsonl"]},
            {"declared_privacy_risk": "medium"},
            {"declared_privacy_risk": {"s01.jsonl": 1}},
        )
        for fields in cases:
            for argv in ([], ["--limit", "3"]):
                with self.subTest(fields=fields, argv=argv):
                    self.assertEqual(self._corpus(None, *argv, pin=self._write_pin(**fields)), 1)
                    self.assertEqual(self._failure(), "PipelineFailure: comparison_pin_unreadable")
                    self.assertEqual(self.calls, [], "refused before any export, cargo, or Docker call")
        # The three fields with their types, and a pin without them.
        usable = {"declared_privacy_risk": {"s01.jsonl": "medium"}}
        absent = {"local_jsonl_dir": _DROP, "session_names": _DROP}
        for fields in (usable, absent):
            with self.subTest(fields=sorted(fields)):
                self._again()
                self.assertEqual(self._corpus(pin=self._write_pin(**fields)), 0, self.stderr.getvalue())

    def test_a_report_with_a_skew_gets_no_check_result(self):
        self.assertEqual(self._corpus(self._harness({"skew": "baseline_quality_floor"})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_skew_with_check_result")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        self.assertFalse(self._report_path().exists())

    def test_a_pass_needs_a_current_check_result(self):
        self.assertEqual(self._corpus(self._harness({"emit": False})), 1)
        self.assertEqual(self._failure(), f"PipelineFailure: check_result_missing:{_COMPARE_LOCAL}")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        self.assertFalse(self._report_path().exists())
        # A result of an earlier time, of another run, or of another tree is
        # not a current result.
        old = _iso(self.run.started_at - timedelta(hours=1))
        for overrides, label in (
            ({"observed_at": old}, "check_result_stale"),
            ({"run_id": "q00000000"}, "check_result_foreign_run"),
            ({"code_revision_hash": _fake_hash("another-tree")}, "check_result_foreign_revision"),
            ({"status": "fail"}, f"check_result_failed:{_COMPARE_LOCAL}"),
            ({"package_hash": None}, f"check_result_digest_missing:{_COMPARE_LOCAL}"),
        ):
            with self.subTest(label=label):
                self._again()
                self.assertEqual(self._corpus(self._harness({"result_overrides": overrides})), 1)
                self.assertEqual(self._failure(), f"PipelineFailure: {label}")
                self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())

    def test_a_pass_needs_a_result_and_evidence_that_agree_with_the_report(self):
        cases = [
            ({"result_overrides": {key: _fake_hash(f"another-{key}")}}, "comparison_report_package_mismatch")
            for key in ("package_hash", "configuration_digest", "dependency_digest")
        ]
        cases += [
            ({"evidence_overrides": overrides}, "comparison_evidence_mismatch")
            for overrides in (
                {"traces": 9},
                {"equal": 9},
                {"permitted": 1},
                {"unexplained": 1},
                {"records_hash": _fake_hash("other-records")},
                {"report_hash": _fake_hash("other-report")},
                {"fixtures": 10},
            )
        ]
        for behavior, label in cases:
            with self.subTest(behavior=behavior):
                self._again()
                self.assertEqual(self._corpus(self._harness(behavior)), 1)
                self.assertEqual(self._failure(), f"PipelineFailure: {label}")
                self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())

        # The evidence names the bytes of the report file.
        def tampering(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
            _write_compare_outputs(env)
            report_path = Path(env["TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH"])
            report_path.write_bytes(report_path.read_bytes() + b" ")

        self._again()
        self.assertEqual(self._corpus(tampering), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_evidence_mismatch")

    def test_the_database_guard_applies(self):
        def low_invoke(calls):
            def fake_invoke(command, *, env, capture=False, input_text=None, log_path=None):
                calls.append(("invoke", list(command), input_text))
                if not capture:
                    return 0, None
                return 0, "3" if "xact_commit" in (input_text or "") else "42"

            return fake_invoke

        with mock.patch.dict(globals(), {"_environment_invoke": low_invoke}):
            code = self._corpus()
        self.assertEqual(code, 1)
        self.assertEqual(self._failure(), "PipelineFailure: database_check_executed_nothing:compare_run")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        self.assertFalse(self._report_path().exists())

    def test_self_test_runs_four_scenarios(self):
        code = self._compare(["--self-test"], self._harness(**self._SELF_TEST))

        self.assertEqual(code, 0, self.stderr.getvalue())
        exports = self._exports()
        self.assertEqual(
            [(pin_path, name) for _, pin_path, name, _, _ in exports],
            [(pipeline.COMPARE_LOCAL_PIN, "local"), (pipeline.COMPARE_RISK_PIN, "risk")],
        )
        fixtures = environment.ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl"
        for _, _, _, local_dir, release in exports:
            self.assertEqual(Path(local_dir), fixtures)
            self.assertIs(release, False)
        cargo_calls = self._cargo_calls()
        self.assertEqual(
            [call[1] for call in cargo_calls],
            ["compare_self_pass", "compare_self_risk", "compare_self_skew", "compare_self_repeat"],
        )
        order = [call[0] for call in self.calls if call[0] in ("export", "cargo")]
        self.assertEqual(order, ["export", "export", "cargo", "cargo", "cargo", "cargo"])
        environments = [call[4] for call in cargo_calls]
        inputs = [
            tuple(
                env[f"TRACE_COMMONS_PIPELINE_COMPARE_{name}_PATH"] for name in ("BOOTSTRAP", "HOLDOUT", "MANIFEST")
            )
            for env in environments
        ]
        self.assertEqual(inputs[0], inputs[2])
        self.assertEqual(inputs[0], inputs[3])
        self.assertFalse(set(inputs[1]) & set(inputs[0]), "the risk scenario has its own export")
        self.assertTrue(all(str(self.run.run_dir / "compare" / "local") in path for path in inputs[0]))
        self.assertTrue(all(str(self.run.run_dir / "compare" / "risk") in path for path in inputs[1]))
        skews = [env.get("TRACE_COMMONS_PIPELINE_COMPARE_SKEW") for env in environments]
        self.assertEqual(skews, [None, None, "baseline_quality_floor", None])
        outputs = set()
        databases = set()
        for (_, step, cargo_args, test_filter, env, exact, ignored) in cargo_calls:
            self.assertEqual(test_filter, _COMPARE_HARNESS)
            self.assertEqual(cargo_args, _INGEST_ARGS, "the self-test uses the debug build")
            self.assertTrue(exact and ignored)
            self.assertEqual(env["TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID"], _COMPARE_LOCAL)
            self.assertFalse(
                [key for key in env if key.startswith("TRACE_COMMONS_PIPELINE_CHECK_")],
                "a self-test scenario emits no check result",
            )
            self.assertNotIn("TRACE_COMMONS_PIPELINE_COMPARE_LIMIT", env)
            self.assertEqual(
                env["TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH"], str(self.run.run_dir / f"{step}-report.json")
            )
            outputs.update(
                env[f"TRACE_COMMONS_PIPELINE_COMPARE_{name}_PATH"] for name in ("REPORT", "RECORDS", "TIMING")
            )
            outputs.add(env["TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT"])
            databases.add(env["TRACE_COMMONS_PG_TEST_DATABASE_URL"])
        self.assertEqual(len(outputs), 16, "each scenario has its own files")
        self.assertEqual(len(databases), 4, "each scenario has its own database")
        self.assertEqual(len({url.rsplit("/", 1)[0] for url in databases}), 1, "one environment")
        lock = [call for call in self.calls if call[0] == "invoke" and "pipeline_tooling_lock" in (call[2] or "")]
        self.assertEqual(len(lock), 2, "the server lock is taken and released one time")

        self.assertEqual(self.stdout.getvalue().splitlines(), ["PipelineCompareSelfTestOK: scenarios=4"])
        self.assertEqual(results.load_results(self.run), {})
        self.assertFalse(self.local.exists(), "the self-test writes no report under .local")

    def test_self_test_fails_when_an_expected_failure_passes(self):
        risk = self._SELF_TEST["compare_self_risk"]
        skew = self._SELF_TEST["compare_self_skew"]
        three_entries = [
            {"position": position, "trace_hash": _fake_hash(f"trace-{position}"), "fields": ["admission"]}
            for position in (5, 6, 7)
        ]

        def refused(side):
            # The risk pin has 8 traces, and the risk scenario compares each.
            sides = {"baseline": _comparison_side(8), "candidate": _comparison_side(8)}
            sides[side] = _comparison_side(8, admit=7, refused=1)
            return sides

        cases = (
            ({"compare_self_risk": {}}, "compare_self_test_risk_passed"),
            ({"compare_self_skew": {}}, "compare_self_test_skew_passed"),
            ({"compare_self_skew": {**skew, "alignment_lost_position": None}}, "compare_self_test_skew_fields"),
            ({"compare_self_skew": {**skew, "unexplained": {"member": 1}}}, "compare_self_test_skew_fields"),
            ({"compare_self_risk": {**risk, "unexplained": {"admission": 2, "member": 1}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "unexplained": {"privacy_risk": 2}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {"fail": True, "branch_gaps": ["member_true"]}}, "compare_self_test_risk_fields"),
            ({"compare_self_repeat": {"records_digest": _fake_hash("other-records")}}, "compare_self_test_not_deterministic"),
            # The risk scenario has one exact result: the declared high risk
            # as one `admission` difference, the declared medium risk as one
            # permitted pair, in a run that compares each trace and keeps the
            # alignment. A run that stops at the first risk trace, or that
            # finds one of the two, is another result.
            (
                {"compare_self_risk": {"fail": True, "compared": 3, "unexplained": {"admission": 1}, "alignment_lost_position": 2}},
                "compare_self_test_risk_fields",
            ),
            ({"compare_self_risk": {**risk, "permitted": {}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "permitted": {_PERMITTED_RULE: 2}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "unexplained": {"admission": 2}, "permitted": {}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "unexplained": {"admission": 3}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "unexplained_counts": {"admission": 3}}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "compared": 7}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "alignment_lost_position": 7}}, "compare_self_test_risk_fields"),
            (
                {"compare_self_risk": {**risk, "unexplained_total": 3, "equal_count": 4, "unexplained_list": three_entries}},
                "compare_self_test_risk_fields",
            ),
            ({"compare_self_risk": {**risk, "distribution": refused("baseline")}}, "compare_self_test_risk_fields"),
            ({"compare_self_risk": {**risk, "distribution": refused("candidate")}}, "compare_self_test_risk_fields"),
            # A scenario that must pass compares each trace of its pin.
            ({"compare_self_pass": {"compared": 0}}, "compare_self_test_pass_incomplete"),
            ({"compare_self_pass": {"compared": 9}}, "compare_self_test_pass_incomplete"),
            ({"compare_self_repeat": {"compared": 0}}, "compare_self_test_pass_incomplete"),
            # The report of each scenario names the skew of its run, or none.
            # `_compare_once` examines this, so the self-test has no check of
            # its own for it.
            ({"compare_self_skew": {**skew, "skew": None}}, "comparison_report_skew_mismatch"),
            ({"compare_self_pass": {"skew": "baseline_quality_floor"}}, "comparison_report_skew_mismatch"),
            # A scenario that must pass, with a report that names a failure.
            ({"compare_self_pass": {"unexplained": {"admission": 1}}}, "comparison_has_unexplained_differences"),
            ({"compare_self_repeat": {"branch_gaps": ["member_true"]}}, "comparison_gate_branch_not_exercised"),
            ({"compare_self_pass": {"write": False}}, "comparison_report_missing"),
        )
        for changed, label in cases:
            with self.subTest(label=label, step=sorted(changed)):
                self._again()
                code = self._compare(["--self-test"], self._harness(**{**self._SELF_TEST, **changed}))
                self.assertEqual(code, 1)
                self.assertEqual(self._failure(), f"PipelineFailure: {label}")
                self.assertNotIn("PipelineCompareSelfTestOK", self.stdout.getvalue())
        # A pin with no trace: a pass that compared nothing.
        self._again()
        self.sample_count = 0
        self.assertEqual(self._compare(["--self-test"], self._harness(**self._SELF_TEST)), 1)
        self.assertEqual(self._failure(), "PipelineFailure: compare_self_test_pass_incomplete")
        self.sample_count = None
        # A scenario that must pass and fails keeps its step failure, with
        # the scenario's name, and an expected failure that leaves no report
        # does too.
        for step, behavior in (
            ("compare_self_pass", {"fail": True, "unexplained": {"admission": 1}}),
            ("compare_self_repeat", {"fail": True, "write": False}),
            ("compare_self_risk", {"fail": True, "write": False}),
            ("compare_self_skew", {"fail": True, "write": False}),
        ):
            with self.subTest(step=step):
                self._again()
                code = self._compare(["--self-test"], self._harness(**{**self._SELF_TEST, step: behavior}))
                self.assertEqual(code, 101)
                self.assertTrue(self._failure().startswith(f"PipelineFailure: step_failed:{step} exit=101 log="))
                self.assertNotIn("PipelineCompareSelfTestOK", self.stdout.getvalue())

    def test_compare_options_are_validated(self):
        cases = (
            ([], "compare_corpus_or_self_test_required"),
            (["--limit", "3"], "compare_corpus_or_self_test_required"),
            (["--corpus", self.local_pin, "--self-test"], "compare_corpus_and_self_test_conflict"),
            (["--self-test", "--limit", "3"], "compare_self_test_takes_no_option"),
            (["--corpus", self.local_pin, "--limit", "0"], "compare_limit_invalid"),
            (["--corpus", self.local_pin, "--limit", "-2"], "compare_limit_invalid"),
        )
        for argv, label in cases:
            with self.subTest(argv=argv):
                self.assertEqual(self._compare(argv), 1)
                self.assertEqual(self._failure(), f"PipelineFailure: {label}")
                self.assertEqual(self.calls, [], "refused before any export, cargo, or Docker call")
        # `compare` has no archive option and no build option.
        for option in (["--archive"], ["--release"], ["--limit", "three"]):
            with self.subTest(option=option):
                with self.assertRaises(SystemExit) as ctx:
                    self._compare(["--corpus", self.local_pin, *option])
                self.assertEqual(ctx.exception.code, 2)
                self.assertEqual(self.calls, [])

    def test_an_alignment_loss_fails_with_its_label(self):
        lost = {"compared": 5, "unexplained": {"member": 1}, "alignment_lost_position": 4}
        self.assertEqual(self._corpus(self._harness({"fail": True, **lost})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_alignment_lost")
        self.assertEqual(
            self.stdout.getvalue().splitlines(),
            [
                "PipelineCompareReport: unexplained=1 alignment_lost=4 partial=true "
                f"run={self.run.run_id} report=pipeline-comparison-{_COMPARE_LOCAL}-partial.json"
            ],
        )
        report = json.loads(self._report_path(partial=True).read_text())
        self.assertEqual((report["partial"], report["alignment_lost_position"]), (True, 4))
        self.assertIn("position 4", self._report_path(partial=True).with_suffix(".md").read_text())
        # Position 0 is a position.
        self._again()
        first = {"compared": 1, "unexplained": {"member": 1}, "alignment_lost_position": 0}
        self.assertEqual(self._corpus(self._harness({"fail": True, **first})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_alignment_lost")
        self.assertIn("alignment_lost=0 ", self.stdout.getvalue())
        # A harness that passes with such a report is refused with the same label.
        for behavior in (lost, {"compared": 5, "alignment_lost_position": 4}):
            with self.subTest(behavior=behavior):
                self._again()
                self.assertEqual(self._corpus(self._harness(behavior)), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_alignment_lost")
                self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())

    def test_a_refused_receipt_fails_with_its_label(self):
        def distribution(side, compared=10):
            sides = {"baseline": _comparison_side(compared), "candidate": _comparison_side(compared)}
            sides[side] = _comparison_side(compared, admit=compared - 1, refused=1)
            return sides

        refused = {"unexplained": {"receipt_code": 1}, "distribution": distribution("candidate")}
        self.assertEqual(self._corpus(self._harness({"fail": True, **refused})), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_receipt_refused")
        self.assertIn("PipelineCompareReport: unexplained=1 alignment_lost=none", self.stdout.getvalue())
        # A harness that passes with such a report and a check result is
        # refused with the same label, for each side.
        for side in ("candidate", "baseline"):
            with self.subTest(side=side):
                self._again()
                self.assertEqual(self._corpus(self._harness({"distribution": distribution(side)})), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_receipt_refused")
                self.assertTrue((self.run.results_dir / f"{_COMPARE_LOCAL}.result.json").is_file())
                self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        # The order is alignment, refusal, unexplained, branch.
        both = {
            **refused,
            "distribution": distribution("candidate", 5),
            "compared": 5,
            "alignment_lost_position": 4,
            "branch_gaps": ["member_true"],
        }
        for behavior in ({"fail": True, **both}, both):
            with self.subTest(fail=behavior.get("fail", False)):
                self._again()
                self.assertEqual(self._corpus(self._harness(behavior)), 1)
                self.assertEqual(self._failure(), "PipelineFailure: comparison_alignment_lost")
        self._again()
        unexplained_and_gap = {"fail": True, "unexplained": {"admission": 1}, "branch_gaps": ["member_true"]}
        self.assertEqual(self._corpus(self._harness(unexplained_and_gap)), 1)
        self.assertEqual(self._failure(), "PipelineFailure: comparison_has_unexplained_differences")
        # A failed step with a report that names no failure keeps the step
        # failure (a driver error, for example).
        self._again()
        self.assertEqual(self._corpus(self._harness({"fail": True})), 101)
        self.assertTrue(self._failure().startswith("PipelineFailure: step_failed:compare_run exit=101 log="))
        self.assertIn("PipelineCompareReport: unexplained=0 alignment_lost=none", self.stdout.getvalue())

    def test_a_corpus_run_uses_the_optimized_build(self):
        self.assertEqual(self._corpus(), 0, self.stderr.getvalue())
        [(_, _, cargo_args, _, _, _, _)] = self._cargo_calls()
        self.assertEqual(cargo_args, (*_INGEST_ARGS, "--release"))
        [(_, _, _, _, release)] = self._exports()
        self.assertIs(release, True)

    def test_a_tree_edited_during_the_run_is_refused(self):
        self.tree_edited = True
        self.assertEqual(self._corpus(), 1)
        self.assertEqual(self._failure(), "PipelineFailure: code_revision_changed")
        self.assertNotIn("PipelineCompareOK", self.stdout.getvalue())
        self.assertEqual(self._local_files(), [], "a refused run leaves no report that reads as a pass")


if __name__ == "__main__":
    unittest.main()
