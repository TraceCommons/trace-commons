#!/usr/bin/env python3
"""The operator tooling entry point for the versioned pipeline.

Python 3 standard library only. Every subcommand builds one `Run`, runs its
handler, and on failure prints a single label-only line to standard error --
never a child command line, its environment, or its output (repo
convention: hash-only, label-only operational surfaces).
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import re
import secrets
import shutil
import sys
from pathlib import Path
from typing import NamedTuple, Optional

from pipeline_tooling.cargo import cargo_test
from pipeline_tooling.catalog import CATALOG_NAME, update_catalog
from pipeline_tooling.checks import (
    CONTRACTS_STEPS,
    INVENTORY_STEP,
    MIGRATION_ATOMICITY_STEP,
    REQUIRED_CORPUS_CHECK_IDS,
    REQUIRED_DATABASE_CHECKS,
    RESTORE_CHECK_ID,
    RUNTIME_STEPS,
    TEST_CHECKS,
    CheckSpec,
    required_specs,
)
from pipeline_tooling.corpus import (
    DEFAULT_CORPUS,
    PIN_SCHEMA,
    export_hf_corpus,
    load_direct_corpus,
    load_pin,
    markdown,
    validate_report,
)
from pipeline_tooling.environment import ROOT, Environment, Run, child_environment, run_child
from pipeline_tooling.errors import StepFailed, ToolingError, require
from pipeline_tooling.files import atomic_write, sha256_digest
from pipeline_tooling.report import SAFE_BLOCKERS, corpus_run_input, write_report
from pipeline_tooling.results import load_results, require_current_pass_results, validate_evidence

# Where routine outputs go (the latest bounded report of each kind), and the
# catalog `--archive` writes (P4-D19). One name so the self-tests can move it.
LOCAL_DIR = ROOT / ".local"

# The ingest test binary hosts the shared app in-process (P4-D3): `run` and
# `package` start one exact ignored test in it.
INGEST_TEST_ARGS = ("-p", "trace-commons-server", "--bin", "trace-commons-ingest")
CORPUS_HARNESS = "tests::pipeline_corpus_pg_tests::pipeline_corpus_run"
PACKAGE_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_package_write"
BUNDLES = ("minimal", "compatibility")

# `restore-drill`: the two ignored tests around the dump, the restore, and
# the artifact copy, and the one check the resume emits.
RESTORE_SEED = "tests::pipeline_restore_pg_tests::pipeline_restore_seed"
RESTORE_RESUME = "tests::pipeline_restore_pg_tests::pipeline_restore_resume"
RESTORE_FINGERPRINT_SCHEMA = "trace_commons.pipeline_restore_fingerprint.v1"
RESTORE_SAFE_BLOCKER = "filesystem_restore_local_only"
_RESTORE_FINGERPRINT_HASHES = (
    "database_fingerprint",
    "artifact_fingerprint",
    "index_entry_set_hash",
    "pending_run_id_hash",
)
# The seed's counts: its adapter requests, and the completed run's
# settlement legs and Trace Credit ledger events (the resume requires the
# pending run to reach the same two).
_RESTORE_FINGERPRINT_COUNTS = (
    "adapter_request_count",
    "completed_settlement_count",
    "completed_credit_event_count",
)

# `qualify`: the contract test manifest whose bytes it hashes (ruling T11-2),
# and its three corpus runs, in `REQUIRED_CORPUS_CHECK_IDS` order, as
# `(bundle, corpus)`. The HF run uses the local fixture pin (ruling HF-1).
CONTRACT_MANIFEST = ROOT / "docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json"
HF_LOCAL_PIN = ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-local.json"
QUALIFY_CORPUS_RUNS = (
    ("minimal", DEFAULT_CORPUS),
    ("compatibility", DEFAULT_CORPUS),
    ("compatibility", HF_LOCAL_PIN),
)

_HASH = re.compile(r"sha256:[a-f0-9]{64}\Z")
_KEY_ID = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")
_FAILURE_LABEL = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")

# A bare `scheme://` anywhere in an argument. `--postgres-admin-url` is the
# one flag allowed to carry a URL; every other argument that looks like one
# is refused before argparse ever sees it.
_URL_LIKE = re.compile(r"[a-zA-Z][a-zA-Z0-9+.-]*://")


def _reject_url_like_arguments(argv):
    skip_next = False
    for token in argv:
        if skip_next:
            skip_next = False
            continue
        if token == "--postgres-admin-url":
            skip_next = True
            continue
        if token.startswith("--postgres-admin-url="):
            continue
        if _URL_LIKE.search(token):
            raise ToolingError("argument_looks_like_url")


def build_parser():
    parser = argparse.ArgumentParser(prog="pipeline.py")
    subparsers = parser.add_subparsers(dest="command", required=True)

    test_parser = subparsers.add_parser("test", help="Run the pipeline qualification test groups")
    test_parser.add_argument(
        "--check",
        dest="checks",
        action="append",
        choices=tuple(TEST_CHECKS),
        help="Run only this check group (repeatable). Default: contracts and runtime.",
    )
    test_parser.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    test_parser.set_defaults(handler=run_test)

    run_parser = subparsers.add_parser(
        "run", help="Run a pipeline corpus through the shared ingest app over real HTTP"
    )
    run_parser.add_argument("--bundle", choices=BUNDLES, default=None, help="Serve this built-in bundle.")
    run_parser.add_argument("--package", default=None, help="Serve this signed bundle package instead.")
    run_parser.add_argument("--trusted-key", dest="trusted_key", default=None, help="The package's trusted key.")
    run_parser.add_argument(
        "--corpus",
        default=str(DEFAULT_CORPUS),
        help="A corpus (trace_commons.pipeline_corpus.v1) or an HF pin descriptor.",
    )
    run_parser.add_argument(
        "--corpus-digest", dest="corpus_digest", default=None, help="Refuse a corpus file with another sha256."
    )
    run_parser.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    run_parser.add_argument("--archive", action="store_true", help="Also archive the report in the lab catalog.")
    run_parser.set_defaults(handler=run_corpus)

    package_parser = subparsers.add_parser("package", help="Build and sign a bundle package (this does not qualify it)")
    package_parser.add_argument("--bundle", choices=BUNDLES, required=True)
    package_parser.add_argument("--output", required=True, help="Where to write the signed package.")
    package_parser.add_argument(
        "--public-key-output", dest="public_key_output", required=True, help="Where to write the trusted key."
    )
    package_parser.add_argument(
        "--signing-key", dest="signing_key", default=None, help="Ed25519 PKCS#8 DER. Omit for a disposable key."
    )
    package_parser.add_argument("--key-id", dest="key_id", default=None, help="The signing key's id.")
    package_parser.set_defaults(handler=run_package)

    restore_parser = subparsers.add_parser(
        "restore-drill",
        help="Dump and restore a seeded pipeline database and resume its pending run (local evidence)",
    )
    restore_parser.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    restore_parser.set_defaults(handler=restore_drill)

    qualify_parser = subparsers.add_parser(
        "qualify", help="Run every required pipeline check and write one qualification report (local evidence)"
    )
    qualify_parser.add_argument(
        "--archive", action="store_true", help="Also archive the report and its records in the lab catalog."
    )
    qualify_parser.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    qualify_parser.set_defaults(handler=qualify)

    return parser


def parse_args(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    _reject_url_like_arguments(argv)
    return build_parser().parse_args(argv)


def _run_plain_step(run, step):
    env = child_environment({})
    if step.kind == "python":
        run_child(run, step.step, list(step.argv), env)
    else:
        cargo_test(run, step.step, step.cargo_args, step.test_filter, env, exact=step.exact, ignored=step.ignored)


def _run_postgres_step(run, environment, step):
    scenario = environment.scenario(step.step)
    extra = {}
    if step.needs_upgrade_db:
        extra["TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"] = scenario.upgrade_url
        check_databases = (scenario.upgrade_database,)
    else:
        extra["TRACE_COMMONS_PG_TEST_DATABASE_URL"] = scenario.runtime_url
        check_databases = (scenario.pilot_database,) if step.needs_pilot_check else (scenario.runtime_database,)
    if step.needs_login_resolver:
        extra["TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL"] = scenario.login_resolver_url

    env = child_environment(extra)
    cargo_test(run, step.step, step.cargo_args, step.test_filter, env, exact=step.exact, ignored=step.ignored)

    committed = scenario.committed_transactions(*check_databases)
    require(committed >= 5, f"database_check_executed_nothing:{step.step}")


def run_test(args, run):
    requested = args.checks or ["contracts", "runtime"]
    ordered = list(dict.fromkeys(requested))  # de-duplicate, keep order

    for name in ordered:
        if name == "postgres":
            continue
        for step in TEST_CHECKS[name]:
            _run_plain_step(run, step)

    if "postgres" in ordered:
        with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
            for step in TEST_CHECKS["postgres"]:
                _run_postgres_step(run, environment, step)


def _corpus_source(args):
    """`(bundle, package, trusted_key)`: exactly one of a built-in bundle or
    a signed package with its trusted key."""
    has_package = args.package is not None or args.trusted_key is not None
    require(not (args.bundle is not None and has_package), "corpus_package_and_bundle_conflict")
    if has_package:
        require(args.package is not None and args.trusted_key is not None, "corpus_package_and_key_required")
        return None, Path(args.package).resolve(), Path(args.trusted_key).resolve()
    require(args.bundle is not None, "corpus_bundle_or_package_required")
    return args.bundle, None, None


def _corpus_partitions(run, corpus_path, expected_digest):
    """`(is_pin, [partition paths])`. A pin descriptor goes through the HF
    export first (bootstrap, then holdout); any other file is one direct
    corpus. `expected_digest` checks the bytes of the file given."""
    raw = corpus_path.read_bytes()
    require(expected_digest is None or sha256_digest(raw) == expected_digest, "corpus_digest_mismatch")
    try:
        value = json.loads(raw)
    except ValueError as error:
        raise ToolingError("corpus_not_json") from error
    if isinstance(value, dict) and value.get("schema") == PIN_SCHEMA:
        pin = load_pin(corpus_path)
        # A local fixture pin names its JSONL directory (no network); a pin
        # without one exports from the HF dataset it names.
        local = pin.get("local_jsonl_dir")
        local_dir = ROOT / local if local else None
        exported = export_hf_corpus(run, corpus_path, child_environment({}), local_dir=local_dir)
        return True, [Path(path) for path in exported]
    return False, [corpus_path]


def _write_local_report(label, report_bytes, report):
    """The latest report of this kind under `.local/`, beside its Markdown
    view. The JSON bytes are the harness's own, so they keep the hash the
    check evidence names."""
    json_path = LOCAL_DIR / f"pipeline-{label}-corpus-report.json"
    atomic_write(json_path, report_bytes)
    atomic_write(json_path.with_suffix(".md"), markdown(report).encode())
    return json_path


def _read_json(path, label):
    """The JSON value in `path`; an unreadable or malformed file fails with
    `label`, never a traceback."""
    try:
        return json.loads(Path(path).read_bytes())
    except (OSError, ValueError) as error:
        raise ToolingError(label) from error


def _keep_failed_report(report_path, label):
    """After a failed harness run: keep a report that still validates (it
    names each fixture's mismatches) and say where it is. Nothing here may
    replace the harness's own failure: a missing, malformed, or invalid
    report, or a write that fails, is left in the run directory silently,
    and the caller re-raises the step failure with its exit code."""
    try:
        report_bytes = report_path.read_bytes()
        report = json.loads(report_bytes)
        validate_report(report)
        json_path = _write_local_report(label, report_bytes, report)
        shown = json_path.relative_to(ROOT) if json_path.is_relative_to(ROOT) else json_path.name
        line = f"PipelineRunReport: failures={report['failure_count']} report={shown}"
    except Exception:  # noqa: BLE001 -- the step failure must win (see above)
        return
    print(line)


class CorpusRun(NamedTuple):
    """A corpus run ready to start: its label (the check id's suffix), the
    source it serves (a built-in bundle, or a package and its trusted key),
    the corpus partition files, and their digests."""

    label: str
    bundle: Optional[str]
    package_path: Optional[Path]
    key_path: Optional[Path]
    partitions: list
    digests: list

    @property
    def check_id(self):
        return f"pipeline_http_corpus_{self.label}"


def _corpus_report_path(run, label):
    return run.run_dir / f"corpus-report-{label}.json"


def prepare_corpus_run(run, bundle, package_path, key_path, corpus_path, expected_digest):
    """Checks the corpus file (and exports an HF pin) before any database
    starts, and names the check the run will emit."""
    is_pin, partitions = _corpus_partitions(run, corpus_path, expected_digest)
    digests = [load_direct_corpus(path)[1] for path in partitions]
    if is_pin:
        label = "hf_local"
    elif package_path is not None:
        label = "package"
    else:
        label = bundle
    return CorpusRun(label, bundle, package_path, key_path, partitions, digests)


def run_corpus_check(run, environment, corpus_run, *, step=None):
    """One corpus run in its own scenario of `environment`: the harness, the
    transaction guard, and every check of its report, result, and evidence.
    Writes the latest report of its kind under `.local/` and returns
    `(local report path, report)`. `step` labels the harness step and the
    scenario (default: the check id). `run` and `qualify` both call this
    (ruling T11-4)."""
    check_id = corpus_run.check_id
    step = step or check_id
    report_path = _corpus_report_path(run, corpus_run.label)
    scenario = environment.scenario(step)
    extra = {
        "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.runtime_url,
        "TRACE_COMMONS_PIPELINE_CORPUS_PATH": str(corpus_run.partitions[0]),
        "TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID": check_id,
        "TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH": str(report_path),
        "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT": str(scenario.artifact_root),
        # One random key per corpus run (P4-D18); only the child sees it.
        "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX": secrets.token_hex(32),
        "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR": str(run.results_dir),
        "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID": run.run_id,
        "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH": run.code_revision_hash,
    }
    if len(corpus_run.partitions) == 2:
        extra["TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH"] = str(corpus_run.partitions[1])
    if corpus_run.package_path is not None:
        extra["TRACE_COMMONS_PIPELINE_CORPUS_PACKAGE_PATH"] = str(corpus_run.package_path)
        extra["TRACE_COMMONS_PIPELINE_CORPUS_TRUSTED_KEY_PATH"] = str(corpus_run.key_path)
    else:
        extra["TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE"] = corpus_run.bundle
    try:
        cargo_test(run, step, INGEST_TEST_ARGS, CORPUS_HARNESS, child_environment(extra), exact=True, ignored=True)
    except StepFailed:
        _keep_failed_report(report_path, corpus_run.label)
        raise
    committed = scenario.committed_transactions(scenario.pilot_database)
    require(committed >= 5, f"database_check_executed_nothing:{step}")

    require(report_path.is_file(), "corpus_report_missing")
    report_bytes = report_path.read_bytes()
    try:
        report = json.loads(report_bytes)
    except ValueError as error:
        raise ToolingError("corpus_report_malformed") from error
    validate_report(report)
    require(report["check_id"] == check_id, "corpus_report_check_mismatch")
    require(
        [section["corpus_digest"] for section in report["partitions"]] == corpus_run.digests,
        "corpus_digest_mismatch",
    )
    require(report["failure_count"] == 0, "corpus_report_has_failures")

    results = load_results(run)
    require_current_pass_results(run, results, {check_id: CheckSpec(check_id, digests_required=True)})
    result = results[check_id]
    require(
        (result.package_hash, result.configuration_digest, result.dependency_digest)
        == (report["package_hash"], report["configuration_digest"], report["dependency_digest"]),
        "corpus_report_package_mismatch",
    )
    evidence = _read_json(run.results_dir / f"{check_id}.evidence.json", "corpus_evidence_malformed")
    validate_evidence(evidence)
    require(
        evidence
        == {
            "fixtures": report["fixture_count"],
            "completed": report["completed_fixture_count"],
            "replay_same_run": report["replay_same_run_count"],
            "changed_content_refused": report["changed_content_refused_count"],
            "tenant_isolation": True,
            "report_hash": sha256_digest(report_bytes),
        },
        "corpus_evidence_mismatch",
    )
    return _write_local_report(corpus_run.label, report_bytes, report), report


def run_corpus(args, run):
    bundle, package_path, key_path = _corpus_source(args)
    corpus_run = prepare_corpus_run(
        run, bundle, package_path, key_path, Path(args.corpus).resolve(), args.corpus_digest
    )
    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
        local_report, report = run_corpus_check(run, environment, corpus_run, step="corpus_run")
    require(set(load_results(run)) == {corpus_run.check_id}, "corpus_check_results_unexpected")
    if args.archive:
        update_catalog(LOCAL_DIR / CATALOG_NAME, local_report)
    print(f"PipelineRunOK: bundle={report['bundle_id']} fixtures={report['fixture_count']}")


def run_package(args, run):
    require((args.signing_key is None) == (args.key_id is None), "package_signing_key_and_key_id_required")
    require(args.key_id is None or _KEY_ID.fullmatch(args.key_id) is not None, "package_key_id_invalid")
    output = Path(args.output).resolve()
    key_output = Path(args.public_key_output).resolve()
    require(output != key_output, "package_outputs_must_differ")
    extra = {
        "TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE": args.bundle,
        "TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT": str(output),
        "TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT": str(key_output),
    }
    if args.signing_key is not None:
        extra["TRACE_COMMONS_PIPELINE_PACKAGE_SIGNING_KEY_PATH"] = str(Path(args.signing_key).resolve())
        extra["TRACE_COMMONS_PIPELINE_PACKAGE_KEY_ID"] = args.key_id
    output.parent.mkdir(parents=True, exist_ok=True)
    key_output.parent.mkdir(parents=True, exist_ok=True)
    cargo_test(run, "package_write", INGEST_TEST_ARGS, PACKAGE_WRITER, child_environment(extra), exact=True, ignored=True)

    signed = _read_json(output, "package_output_invalid")
    package = signed.get("package") if isinstance(signed, dict) else None
    signature = signed.get("signature") if isinstance(signed, dict) else None
    bundle_id = package.get("bundle_id") if isinstance(package, dict) else None
    package_hash = signature.get("package_hash") if isinstance(signature, dict) else None
    require(
        isinstance(bundle_id, str)
        and _HASH.fullmatch(bundle_id) is not None
        and isinstance(package_hash, str)
        and _HASH.fullmatch(package_hash) is not None,
        "package_output_invalid",
    )
    require(key_output.is_file(), "package_trusted_key_missing")
    print(f"PipelinePackageOK: bundle={bundle_id} package={package_hash}")


def _artifact_files(root):
    """Every file under `root`, keyed by its `/`-separated relative path."""
    root = Path(root)
    return {path.relative_to(root).as_posix(): path for path in root.rglob("*") if path.is_file()}


def artifact_fingerprint(root):
    """The port's artifact fingerprint (`ef97a459:scripts/operator/pipeline-
    backup-restore-smoke.sh` lines 191-205): SHA-256 over each file's
    relative path and the SHA-256 of its bytes, in relative-path string
    order. `artifact_fingerprint` in `pipeline_restore_pg_tests.rs` computes
    the same value (both pin one tree's)."""
    digest = hashlib.sha256()
    files = _artifact_files(root)
    for relative in sorted(files):
        digest.update(relative.encode())
        digest.update(hashlib.sha256(files[relative].read_bytes()).digest())
    return "sha256:" + digest.hexdigest()


def copy_artifacts(source, destination):
    """The filesystem restore: a copy of the seed's artifact root to a new
    directory beside it."""
    shutil.copytree(source, destination)


def require_same_artifact_bytes(source, destination):
    """The copy holds exactly the source's files, byte for byte."""
    source_files, copied_files = _artifact_files(source), _artifact_files(destination)
    require(set(source_files) == set(copied_files), "restore_artifact_bytes_mismatch")
    for relative, path in source_files.items():
        require(path.read_bytes() == copied_files[relative].read_bytes(), "restore_artifact_bytes_mismatch")


def _read_restore_fingerprint(path):
    """The seed's fingerprint file: exactly its schema, four hashes, and
    three positive counts."""
    value = _read_json(path, "restore_fingerprint_invalid")
    require(
        isinstance(value, dict)
        and set(value) == {"schema", *_RESTORE_FINGERPRINT_HASHES, *_RESTORE_FINGERPRINT_COUNTS}
        and value["schema"] == RESTORE_FINGERPRINT_SCHEMA
        and all(
            isinstance(value[key], str) and _HASH.fullmatch(value[key]) is not None
            for key in _RESTORE_FINGERPRINT_HASHES
        )
        and all(type(value[key]) is int and value[key] > 0 for key in _RESTORE_FINGERPRINT_COUNTS),
        "restore_fingerprint_invalid",
    )
    return value


def run_restore_drill(run, environment):
    """One restore drill in its own scenario of `environment`, in this order:
    the seed (on `<db>_pilot`), the dump of `<db>_pilot`, `<db>_restored`
    created in the same cluster, the restore, the artifact copy and its
    byte comparison, then the resume against `<db>_restored`. Checks the
    resume's `pipeline_restore_drill` result against the seed and returns
    the seed's fingerprint. Takes the environment as an argument so that
    `qualify` can run it beside its other checks."""
    scenario = environment.scenario("restore_drill")
    source_root = scenario.artifact_root
    restored_root = source_root.with_name(f"{source_root.name}_restored")
    fingerprint_path = run.run_dir / "restore-fingerprint.json"
    dump_path = run.run_dir / f"{run.run_id}.dump"
    shared = {
        # One random key for both processes (P4-D18): the resume must read
        # the objects the seed wrote. Only the children see it.
        "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX": secrets.token_hex(32),
        "TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH": str(fingerprint_path),
    }

    seed_env = {
        **shared,
        "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.runtime_url,
        "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT": str(source_root),
    }
    cargo_test(
        run, "restore_seed", INGEST_TEST_ARGS, RESTORE_SEED, child_environment(seed_env), exact=True, ignored=True
    )
    require(
        scenario.committed_transactions(scenario.pilot_database) >= 5,
        "database_check_executed_nothing:restore_seed",
    )
    seed = _read_restore_fingerprint(fingerprint_path)

    try:
        environment.dump(scenario.pilot_database, dump_path)
        environment.create_database(scenario.restored_database)
        environment.restore(dump_path, scenario.restored_database)
    finally:
        # Admin-url mode leaves the dump in the run directory; it is not kept.
        dump_path.unlink(missing_ok=True)
    copy_artifacts(source_root, restored_root)
    require_same_artifact_bytes(source_root, restored_root)
    require(
        artifact_fingerprint(restored_root) == seed["artifact_fingerprint"], "restore_artifact_fingerprint_mismatch"
    )

    resume_env = {
        **shared,
        "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.restored_url,
        "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT": str(restored_root),
        "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR": str(run.results_dir),
        "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID": run.run_id,
        "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH": run.code_revision_hash,
    }
    cargo_test(
        run, "restore_resume", INGEST_TEST_ARGS, RESTORE_RESUME, child_environment(resume_env), exact=True, ignored=True
    )
    require(
        scenario.committed_transactions(scenario.restored_database) >= 5,
        "database_check_executed_nothing:restore_resume",
    )

    results = load_results(run)
    require(RESTORE_CHECK_ID in results, f"check_result_missing:{RESTORE_CHECK_ID}")
    require_current_pass_results(
        run, results, {RESTORE_CHECK_ID: CheckSpec(RESTORE_CHECK_ID, digests_required=True)}
    )
    require(RESTORE_SAFE_BLOCKER in results[RESTORE_CHECK_ID].safe_blockers, "restore_safe_blocker_missing")
    evidence = _read_json(run.results_dir / f"{RESTORE_CHECK_ID}.evidence.json", "restore_evidence_malformed")
    validate_evidence(evidence)
    require(
        evidence
        == {
            "database_fingerprint": seed["database_fingerprint"],
            "artifact_fingerprint": seed["artifact_fingerprint"],
            "index_entry_set_hash": seed["index_entry_set_hash"],
            "pending_runs_resumed": 1,
            "duplicate_effects": 0,
        },
        "restore_evidence_mismatch",
    )
    return seed


def restore_drill(args, run):
    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
        seed = run_restore_drill(run, environment)
    require(set(load_results(run)) == {RESTORE_CHECK_ID}, "restore_check_results_unexpected")
    print(
        f"PipelineRestoreOK: database={seed['database_fingerprint']} "
        f"artifacts={seed['artifact_fingerprint']} index={seed['index_entry_set_hash']} "
        f"legs_per_run={seed['completed_settlement_count']} "
        f"credit_events_per_run={seed['completed_credit_event_count']} "
        "pending_runs_resumed=1 duplicate_effects=0"
    )
    print(
        f"PipelineRestoreScope: {RESTORE_SAFE_BLOCKER} -- the artifact restore is a local "
        "filesystem copy, local evidence only, not a remote object-store restore"
    )


def run_binding_checks(run):
    """The checks `qualify` requires by exit status before any database
    check: every `pipeline.py test` step of `contracts` and `runtime` (the
    runtime group holds the tooling self-tests), with the deployment
    inventory also writing `<run dir>/inventory.json` (ruling T11-3).
    Returns that inventory's `inventory_digest`."""
    inventory_path = run.run_dir / "inventory.json"
    for step in (*CONTRACTS_STEPS, *RUNTIME_STEPS):
        if step.step == INVENTORY_STEP:
            step = dataclasses.replace(step, argv=(*step.argv, "--output", str(inventory_path)))
        _run_plain_step(run, step)
    inventory = _read_json(inventory_path, "inventory_output_invalid")
    digest = inventory.get("inventory_digest") if isinstance(inventory, dict) else None
    require(isinstance(digest, str) and _HASH.fullmatch(digest) is not None, "inventory_output_invalid")
    return digest


def run_database_check(run, scenario, check):
    """One required database check in its own scenario: its exact test,
    pointed at the scenario's database and the run's result directory, then
    the transaction guard on the database the test ran in, then a current
    pass result for its check id."""
    extra = {
        "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR": str(run.results_dir),
        "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID": run.run_id,
        "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH": run.code_revision_hash,
    }
    if check.database == "upgrade":
        extra["TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"] = scenario.upgrade_url
        database = scenario.upgrade_database
    else:
        extra["TRACE_COMMONS_PG_TEST_DATABASE_URL"] = scenario.runtime_url
        database = scenario.pilot_database if check.database == "pilot" else scenario.runtime_database
    env = child_environment(extra)
    cargo_test(run, check.check_id, check.cargo_args, check.test_name, env, exact=True, ignored=check.ignored)
    require(scenario.committed_transactions(database) >= 5, f"database_check_executed_nothing:{check.check_id}")
    require_current_pass_results(run, load_results(run), {check.check_id: CheckSpec(check.check_id, check.digests)})


def corpus_records(run):
    """What `qualify --archive` archives beside its report (ruling T11-1):
    the run directory's copy of each required corpus report, and the HF
    export's manifest."""
    labels = [check_id.removeprefix("pipeline_http_corpus_") for check_id in REQUIRED_CORPUS_CHECK_IDS]
    return [*(_corpus_report_path(run, label) for label in labels), run.run_dir / "hf" / "source-manifest.json"]


def _run_required_checks(args, run, inputs):
    """Every required check, in order, filling `inputs` as it goes; returns
    the run's results once each required check has a current pass."""
    require(CONTRACT_MANIFEST.is_file(), "contract_manifest_missing")
    inputs["contract_manifest_digest"] = sha256_digest(CONTRACT_MANIFEST.read_bytes())
    inputs["inventory_digest"] = run_binding_checks(run)
    corpus_runs = [
        prepare_corpus_run(run, bundle, None, None, corpus_path, None) for bundle, corpus_path in QUALIFY_CORPUS_RUNS
    ]
    require(
        [corpus_run.check_id for corpus_run in corpus_runs] == list(REQUIRED_CORPUS_CHECK_IDS),
        "qualify_corpus_checks_invalid",
    )
    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
        _run_postgres_step(run, environment, MIGRATION_ATOMICITY_STEP)
        for check in REQUIRED_DATABASE_CHECKS:
            run_database_check(run, environment.scenario(check.check_id), check)
        for corpus_run in corpus_runs:
            _, report = run_corpus_check(run, environment, corpus_run)
            inputs["corpus_runs"].append(corpus_run_input(report))
        run_restore_drill(run, environment)
    # The environment is gone by now: a scenario database, the lock
    # database, or the container that outlived it fails the qualification.
    require(not run.cleanup_failed, "cleanup_failed")
    results = load_results(run)
    require_current_pass_results(run, results, required_specs())
    return results


def _write_failed_report(run, inputs, label):
    """The report of a failed qualification: `status: fail`, the safe label,
    and whatever results and inputs the run reached. Nothing here may
    replace the original failure, so a report that cannot be written is
    skipped silently."""
    if _FAILURE_LABEL.fullmatch(label) is None:
        label = "qualify_failed"
    try:
        try:
            results = load_results(run)
        except ToolingError:
            results = {}
        write_report(run, results, inputs, failure=label, local_dir=LOCAL_DIR)
    except Exception:  # noqa: BLE001 -- the original failure must win (see above)
        return


def _shown(path):
    return path.relative_to(ROOT) if path.is_relative_to(ROOT) else path.name


def qualify(args, run):
    """Runs every required check (binding checks by exit status, then each
    database check, corpus run, and the restore drill in its own scenario of
    one environment), requires a current pass result for each, and writes
    one bounded report. A failure anywhere still writes the report, with
    `status: fail` and the safe label, then raises."""
    inputs = {
        "code_revision_hash": run.code_revision_hash,
        "contract_manifest_digest": None,
        "inventory_digest": None,
        "corpus_runs": [],
    }
    catalog_path = None
    try:
        results = _run_required_checks(args, run, inputs)
        report_path = write_report(run, results, inputs, local_dir=LOCAL_DIR)
        if args.archive:
            catalog_path = LOCAL_DIR / CATALOG_NAME
            update_catalog(catalog_path, report_path, records=corpus_records(run))
    except ToolingError as error:
        _write_failed_report(run, inputs, str(error))
        raise
    except Exception:
        _write_failed_report(run, inputs, "qualify_internal_error")
        raise
    line = f"PipelineQualificationOK: report={_shown(report_path)} checks={len(results)}"
    if catalog_path is not None:
        line += f" catalog={_shown(catalog_path)}"
    print(line)
    print(
        "PipelineQualificationScope: production_promotion_ready=false blockers="
        + ",".join(SAFE_BLOCKERS)
        + " -- local evidence only, not a production promotion"
    )


def main(argv=None):
    args = parse_args(argv)
    run = Run.create()
    primary = 0
    primary_label = None
    try:
        args.handler(args, run)
    except StepFailed as failure:
        print(
            f"PipelineFailure: {failure} exit={failure.exit_code} "
            f"log={failure.log_path.relative_to(ROOT)}",
            file=sys.stderr,
        )
        primary = failure.exit_code or 1
    except ToolingError as error:
        print(f"PipelineFailure: {error}", file=sys.stderr)
        primary = 1
        primary_label = str(error)
    except KeyboardInterrupt:
        primary = 130
    cleanup = 1 if run.cleanup_failed else 0
    # `qualify` raises `cleanup_failed` itself (its report must say so);
    # the line is printed once.
    if cleanup and primary_label != "cleanup_failed":
        print("PipelineFailure: cleanup_failed", file=sys.stderr)
    return primary if primary != 0 else cleanup


if __name__ == "__main__":
    raise SystemExit(main())
