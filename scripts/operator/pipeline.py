#!/usr/bin/env python3
"""The operator tooling entry point for the versioned pipeline.

Python 3 standard library only. Every subcommand builds one `Run`, runs its
handler, and on failure prints a single label-only line to standard error --
never a child command line, its environment, or its output (repo
convention: hash-only, label-only operational surfaces).

`compare` sends the traces of one pin through the old gate path of `main`
and through the versioned pipeline, and reports each difference
(`pipeline_tooling.comparison`).
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import re
import secrets
import shutil
import stat
import sys
import time
from pathlib import Path
from typing import NamedTuple, Optional

from pipeline_tooling import promote
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
    corpus_check_spec,
    required_specs,
)
from pipeline_tooling.comparison import (
    ALIGNMENT_LABEL,
    BRANCH_LABEL,
    PIN_DIGEST_FIELDS,
    REFUSED_LABEL,
    SIDES,
    UNEXPLAINED_LABEL,
    export_compare_corpus,
    file_digest,
    validate_comparison_report,
)
from pipeline_tooling.comparison import markdown as comparison_markdown
from pipeline_tooling.corpus import (
    DEFAULT_CORPUS,
    PIN_SCHEMA,
    evidence_digests,
    export_hf_corpus,
    load_direct_corpus,
    load_pin,
    markdown,
    validate_report,
)
from pipeline_tooling.environment import ROOT, Environment, Run, child_environment, run_child
from pipeline_tooling.errors import StepFailed, ToolingError, require
from pipeline_tooling.files import atomic_write, sha256_digest
from pipeline_tooling.report import REPORT_NAME, RUN_REPORT_NAME, SAFE_BLOCKERS, corpus_run_input, write_report
from pipeline_tooling.results import (
    load_results,
    require_attestations,
    require_current_pass_results,
    validate_evidence,
)

# Where routine outputs go (the latest bounded report of each kind), and the
# catalog `--archive` writes (P4-D19). One name so the self-tests can move it.
LOCAL_DIR = ROOT / ".local"

# The ingest test binary hosts the shared app in-process (P4-D3): `run` and
# `package` start one exact ignored test in it.
INGEST_TEST_ARGS = ("-p", "trace-commons-server", "--bin", "trace-commons-ingest")
CORPUS_HARNESS = "tests::pipeline_corpus_pg_tests::pipeline_corpus_run"
PACKAGE_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_package_write"
# `qualify --signing-key` and `keygen` (P5-D13): the signing step and the key
# writer, two more ignored tests beside the package writer.
ATTESTATION_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_check_attestations_write"
KEY_WRITER = "tests::pipeline_corpus_pg_tests::pipeline_signing_key_write"
BUNDLES = ("minimal", "compatibility")

# The maximum age a signed result carries when `--evidence-max-age-seconds` is
# not given, and the longest the server accepts
# (`QUALIFICATION_EVIDENCE_DEFAULT_MAX_AGE_SECONDS` and
# `QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS`; a self-test reads both out of
# the Rust source).
DEFAULT_EVIDENCE_AGE_SECONDS = 24 * 60 * 60
EVIDENCE_AGE_CEILING_SECONDS = 7 * 24 * 60 * 60

# The signing step writes into this directory of the run directory; the files
# move to the results directory only once every later check has passed.
ATTESTATION_STAGING = "attestations-staging"
# A failed run that could not remove what it signed: shown beside the original
# failure label (`<original>.<this label>` in the failure report).
DISCARD_FAILED_LABEL = "check_attestation_discard_failed"

# Commands that keep nothing: their run directory is removed when they succeed
# (or fail before they wrote anything).
EPHEMERAL_COMMANDS = frozenset({"revision", "keygen"})

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
    "runtime_privilege_set_hash",
    "tenant_fingerprint",
    "rls_policy_set_hash",
    "rls_flag_set_hash",
)
# The seed's counts: its adapter requests, the completed run's settlement
# legs and Trace Credit ledger events (the resume requires the pending run
# to reach the same two), the trace tables it found isolated, the RLS
# policies in the schema, the tables whose RLS flags it hashed, the runtime
# login's privileges, the tenants with rows (at least two), and the hashed
# audit events `main`'s verifier accepted.
_RESTORE_FINGERPRINT_COUNTS = (
    "adapter_request_count",
    "completed_settlement_count",
    "completed_credit_event_count",
    "rls_table_count",
    "rls_policy_count",
    "rls_flag_table_count",
    "runtime_privilege_count",
    "tenant_count",
    "audit_event_count",
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

# `compare`: the ignored test that sends each trace of a pin through the old
# gate path and through the pipeline, and the two pins of `--self-test`.
COMPARE_HARNESS = "tests::pipeline_compare_pg_tests::pipeline_compare_run"
COMPARE_LOCAL_PIN = ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local.json"
COMPARE_RISK_PIN = ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local-risk.json"

_HASH = re.compile(r"sha256:[a-f0-9]{64}\Z")
_KEY_ID = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")
# A public key: 32 bytes, 43 characters of unpadded base64url.
_PUBLIC_KEY = re.compile(r"[A-Za-z0-9_-]{43}\Z")
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

    compare_parser = subparsers.add_parser(
        "compare", help="Send one pin through the old gate path and the pipeline and report each difference"
    )
    compare_parser.add_argument(
        "--corpus", default=None, help="A comparison pin (an HF pin descriptor with with_events: true)."
    )
    compare_parser.add_argument(
        "--self-test",
        dest="self_test",
        action="store_true",
        help="Run the four scenarios that show that the comparison finds a difference. Not evidence.",
    )
    compare_parser.add_argument(
        "--limit", type=int, default=None, help="Compare only the first N traces (a partial run; not evidence)."
    )
    compare_parser.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    compare_parser.set_defaults(handler=run_compare)

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
    qualify_parser.add_argument(
        "--signing-key",
        dest="signing_key",
        default=None,
        help="After a passing run, sign each result with this Ed25519 PKCS#8 DER key (needs --signing-key-id).",
    )
    qualify_parser.add_argument(
        "--signing-key-id",
        dest="signing_key_id",
        default=None,
        help="The id the check trust store holds the signing key's public key under.",
    )
    qualify_parser.add_argument(
        "--evidence-max-age-seconds",
        dest="evidence_max_age_seconds",
        type=int,
        default=DEFAULT_EVIDENCE_AGE_SECONDS,
        help=(
            "How long a signed result stays current (default 86400; the server refuses more than 604800). "
            "Only used with --signing-key."
        ),
    )
    qualify_parser.set_defaults(handler=qualify)

    keygen_parser = subparsers.add_parser(
        "keygen", help="Generate a check-signing key and its trusted key (this trusts it nowhere)"
    )
    keygen_parser.add_argument(
        "--output", required=True, help="Where to write the new Ed25519 PKCS#8 DER key (mode 0600; never overwritten)."
    )
    keygen_parser.add_argument("--key-id", dest="key_id", required=True, help="The new key's id.")
    keygen_parser.add_argument(
        "--trusted-key-output",
        dest="trusted_key_output",
        required=True,
        help="Where to write the trusted key (key id and public key; never overwritten).",
    )
    keygen_parser.set_defaults(handler=keygen)

    revision_parser = subparsers.add_parser(
        "revision", help="Print the code revision hash of the working tree (the one a qualification run records)"
    )
    revision_parser.set_defaults(handler=revision)

    # Spec 2026-10-08 Slice B: the operator-run production checks.
    promote.add_parsers(subparsers, promote.Hooks(signing_options=signing_options, sign=sign_promoted_results))

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
    spec = corpus_check_spec(check_id)
    require_current_pass_results(run, results, {check_id: spec})
    result = results[check_id]
    # The minimal corpus check names no package (P5-D15), so there is
    # nothing to compare; its report keeps the digests of the bundle it served.
    if spec.digests_required:
        require(
            (result.package_hash, result.configuration_digest, result.dependency_digest)
            == (report["package_hash"], report["configuration_digest"], report["dependency_digest"]),
            "corpus_report_package_mismatch",
        )
    evidence = _read_json(run.results_dir / f"{check_id}.evidence.json", "corpus_evidence_malformed")
    validate_evidence(evidence)
    # The corpus loaded and the requests posted (P5-D14): recomputed from the
    # report, so an attestation never signs a value the report does not back.
    corpus_digest, input_digest = evidence_digests(report)
    require(
        evidence
        == {
            "fixtures": report["fixture_count"],
            "completed": report["completed_fixture_count"],
            "replay_same_run": report["replay_same_run_count"],
            "changed_content_refused": report["changed_content_refused_count"],
            "tenant_isolation": True,
            "report_hash": sha256_digest(report_bytes),
            "corpus_digest": corpus_digest,
            "input_digest": input_digest,
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
    run.require_code_revision_unchanged()
    if args.archive:
        update_catalog(LOCAL_DIR / CATALOG_NAME, local_report)
    print(f"PipelineRunOK: bundle={report['bundle_id']} fixtures={report['fixture_count']}")


def _compare_pin(pin_path):
    """`(pin, local directory or None)` of a comparison pin. A file that
    cannot be read, or that is not a JSON object, is
    `comparison_pin_unreadable`. A pin without `with_events: true` is
    `comparison_pin_without_events`: its export has no session events. A
    local fixture pin names its JSONL directory; a pin without one exports
    from the HF dataset that it names.

    Each of the three fields that only a comparison pin has is absent or
    has its type, or the pin is `comparison_pin_unreadable`:
    `local_jsonl_dir` is a string, `session_names` is a list of strings,
    and `declared_privacy_risk` is a map of strings."""
    require(isinstance(_read_json(pin_path, "comparison_pin_unreadable"), dict), "comparison_pin_unreadable")
    pin = load_pin(pin_path)
    require(pin.get("with_events") is True, "comparison_pin_without_events")
    local = pin.get("local_jsonl_dir")
    names = pin.get("session_names")
    risks = pin.get("declared_privacy_risk")
    require(local is None or isinstance(local, str), "comparison_pin_unreadable")
    require(
        names is None or (isinstance(names, list) and all(isinstance(name, str) for name in names)),
        "comparison_pin_unreadable",
    )
    require(
        risks is None or (isinstance(risks, dict) and all(isinstance(risk, str) for risk in risks.values())),
        "comparison_pin_unreadable",
    )
    return pin, (ROOT / local if local else None)


class CompareInput(NamedTuple):
    """One export, ready for the harness: its three files, and what its
    manifest says of the sample (the five digests, and the number of
    traces in the two corpus files)."""

    bootstrap: Path
    holdout: Path
    manifest: Path
    pin: dict
    trace_count: int


def _compare_input(exported):
    """The `CompareInput` of one `export_compare_corpus` result. A manifest
    that cannot be read, or that lacks one of the five digests or a whole
    `sample_count`, is `comparison_manifest_malformed`."""
    bootstrap, holdout, manifest_path = exported
    manifest = _read_json(manifest_path, "comparison_manifest_malformed")
    require(isinstance(manifest, dict), "comparison_manifest_malformed")
    pin = {field: manifest.get(field) for field in PIN_DIGEST_FIELDS}
    trace_count = manifest.get("sample_count")
    require(
        all(isinstance(digest, str) and _HASH.fullmatch(digest) is not None for digest in pin.values())
        and type(trace_count) is int
        and trace_count >= 0,
        "comparison_manifest_malformed",
    )
    return CompareInput(Path(bootstrap), Path(holdout), Path(manifest_path), pin, trace_count)


def _compare_records_path(run, step):
    return run.run_dir / f"{step}-records.jsonl"


def _compare_once(run, environment, exported, check_id, step, *, limit=None, skew=None, release=False, emit=True):
    """One harness run in its own scenario of `environment`. Returns
    `(report or None, StepFailed or None, report bytes or None)`.

    Each file of the run is in the run directory, with the step in its
    name: the harness removes a report that exists at its report path.
    `release` selects Cargo's optimized build (PC-D21). Without `emit`, the
    harness gets none of the three check result variables, so it emits no
    result (`--self-test`: one check id, four runs).

    A harness that fails keeps its failure: the transaction guard does not
    apply, and a report that is missing or not valid is left out. A harness
    that passes must show at least 5 committed transactions, and its
    report, when there is one, must be valid.

    A valid report must be the report of this run, for a harness that
    passes and for one that fails: it names `check_id`
    (`comparison_report_check_mismatch`), the five digests of the export
    manifest (`comparison_report_pin_mismatch`), the manifest's trace count
    (`comparison_report_trace_count_mismatch`), and the skew that this call
    set, or none (`comparison_report_skew_mismatch`). A report with a skew
    never has a check result (`comparison_skew_with_check_result`)."""
    report_path = run.run_dir / f"{step}-report.json"
    scenario = environment.scenario(step)
    extra = {
        "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.runtime_url,
        "TRACE_COMMONS_PIPELINE_COMPARE_BOOTSTRAP_PATH": str(exported.bootstrap),
        "TRACE_COMMONS_PIPELINE_COMPARE_HOLDOUT_PATH": str(exported.holdout),
        "TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH": str(exported.manifest),
        "TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID": check_id,
        "TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH": str(report_path),
        "TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH": str(_compare_records_path(run, step)),
        # The milliseconds of each trace. Not part of the report or of a digest.
        "TRACE_COMMONS_PIPELINE_COMPARE_TIMING_PATH": str(run.run_dir / f"{step}-timing.jsonl"),
        "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT": str(scenario.artifact_root),
        # One random key per harness run (P4-D18); only the child sees it.
        "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX": secrets.token_hex(32),
    }
    if emit:
        extra["TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR"] = str(run.results_dir)
        extra["TRACE_COMMONS_PIPELINE_CHECK_RUN_ID"] = run.run_id
        extra["TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH"] = run.code_revision_hash
    if limit is not None:
        extra["TRACE_COMMONS_PIPELINE_COMPARE_LIMIT"] = str(limit)
    if skew is not None:
        extra["TRACE_COMMONS_PIPELINE_COMPARE_SKEW"] = skew
    # `cargo_test` puts the arguments in its list command and in its run
    # command, so the two use one profile.
    cargo_args = (*INGEST_TEST_ARGS, "--release") if release else INGEST_TEST_ARGS
    failure = None
    try:
        cargo_test(run, step, cargo_args, COMPARE_HARNESS, child_environment(extra), exact=True, ignored=True)
    except StepFailed as error:
        failure = error
    if failure is None:
        committed = scenario.committed_transactions(scenario.pilot_database)
        require(committed >= 5, f"database_check_executed_nothing:{step}")

    if not report_path.is_file():
        return None, failure, None
    report_bytes = report_path.read_bytes()
    try:
        try:
            report = json.loads(report_bytes)
        except ValueError as error:
            raise ToolingError("comparison_report_malformed") from error
        validate_comparison_report(report)
    except ToolingError:
        if failure is None:
            raise
        return None, failure, None
    require(report["check_id"] == check_id, "comparison_report_check_mismatch")
    require(report["pin"] == exported.pin, "comparison_report_pin_mismatch")
    require(report["trace_count"] == exported.trace_count, "comparison_report_trace_count_mismatch")
    # The more exact label first: a skew together with a result of the check.
    require(
        report["skew"] is None or not (run.results_dir / f"{check_id}.result.json").exists(),
        "comparison_skew_with_check_result",
    )
    require(report["skew"] == skew, "comparison_report_skew_mismatch")
    return report, failure, report_bytes


def _compare_failure_label(report):
    """The first failure that a valid report names, or `None`: a pair that
    loses the alignment and a refused pair are also unexplained, and the
    more exact label must win. The harness gives a failure that it kept (a
    driver error, for example) before each of these. The command sees only
    the report, so for a run that such a failure cut short after a
    difference it gives the label of the difference; `partial` then says
    that the run did not complete."""
    if report["alignment_lost_position"] is not None:
        return ALIGNMENT_LABEL
    if any(report["distribution"][side]["refused"] for side in SIDES):
        return REFUSED_LABEL
    if report["unexplained_total"] > 0:
        return UNEXPLAINED_LABEL
    if report["branch_gaps"]:
        return BRANCH_LABEL
    return None


def _require_passing_report(report):
    """After a harness run that passed: it wrote a report
    (`comparison_report_missing`), and the report names no failure."""
    require(report is not None, "comparison_report_missing")
    label = _compare_failure_label(report)
    require(label is None, label)


def _write_compare_report(check_id, report, report_bytes, *, failed=False):
    """The latest report of this check under `.local/`, beside its Markdown
    view. One name for each check id and for a partial run, so a `--limit`
    run or a local-pin run does not replace the report of the full run. The
    JSON bytes are the harness's own, so they keep the hash that the check
    evidence names.

    `failed`: the harness failed. No check result exists for its report, so
    the name of a full report has `-failed`, and the plain name is the name
    of a pass only. A partial report keeps `-partial`."""
    if report["partial"]:
        suffix = "-partial"
    elif failed:
        suffix = "-failed"
    else:
        suffix = ""
    json_path = LOCAL_DIR / f"pipeline-comparison-{check_id}{suffix}.json"
    atomic_write(json_path, report_bytes)
    atomic_write(json_path.with_suffix(".md"), comparison_markdown(report).encode())
    return json_path


def _require_compare_result(run, results, check_id, report, report_bytes):
    """A full run that passed: a current pass result for `check_id` from
    this run, with the package that the report names and with evidence
    equal to the report's counts and to the hashes of the records file
    (`comparison_records_missing` without one) and of the report file."""
    require_current_pass_results(run, results, {check_id: CheckSpec(check_id, digests_required=True)})
    result = results[check_id]
    require(
        (result.package_hash, result.configuration_digest, result.dependency_digest)
        == (report["package_hash"], report["configuration_digest"], report["dependency_digest"]),
        "comparison_report_package_mismatch",
    )
    evidence = _read_json(run.results_dir / f"{check_id}.evidence.json", "comparison_evidence_malformed")
    validate_evidence(evidence)
    try:
        records_hash = file_digest(_compare_records_path(run, "compare_run"))
    except OSError as error:
        raise ToolingError("comparison_records_missing") from error
    require(
        evidence
        == {
            "traces": report["compared_count"],
            "equal": report["equal_count"],
            "permitted": report["permitted_total"],
            "unexplained": 0,
            "records_hash": records_hash,
            "report_hash": sha256_digest(report_bytes),
        },
        "comparison_evidence_mismatch",
    )


def _compare_self_test(args, run):
    """`compare --self-test`: four harness runs of the two local pins in one
    environment, with the debug build. Each run must give its one expected
    result:

    1. The local pin passes, and compares each of its traces
       (`compare_self_test_pass_incomplete`).
    2. The risk pin fails (`compare_self_test_risk_passed`) with its declared
       high risk as one unexplained `admission` difference and its declared
       medium risk as one pair that the rule `medium_risk_privacy_review`
       permits, and nothing else: no other field, no other rule, no refused
       receipt, no alignment loss, and each trace compared
       (`compare_self_test_risk_fields`).
    3. The local pin with the baseline's quality floor skewed fails
       (`compare_self_test_skew_passed`), and its report names the pair at
       which the run stopped and `quality_passed`
       (`compare_self_test_skew_fields`).
    4. The local pin again gives the report of run 1
       (`compare_self_test_not_deterministic`).

    No scenario emits a check result, and no report goes under `.local/`:
    the result is not evidence."""
    check_id = "pipeline_comparison_local"
    exports = {}
    for name, pin_path in (("local", COMPARE_LOCAL_PIN), ("risk", COMPARE_RISK_PIN)):
        _, local_dir = _compare_pin(pin_path)
        exports[name] = _compare_input(
            export_compare_corpus(run, pin_path, child_environment({}), name=name, local_dir=local_dir, release=False)
        )

    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:

        def scenario(step, name, *, skew=None, passed_label=None):
            """The report of one scenario. Without `passed_label` the
            harness must pass and compare each trace of a pin that has
            one. With it, the harness must fail and leave a valid report."""
            report, failure, _ = _compare_once(
                run, environment, exports[name], check_id, step, skew=skew, emit=False
            )
            if passed_label is None:
                if failure is not None:
                    raise failure
                _require_passing_report(report)
                require(
                    not report["partial"] and report["compared_count"] == report["trace_count"] > 0,
                    "compare_self_test_pass_incomplete",
                )
                return report
            require(failure is not None, passed_label)
            if report is None:
                raise failure
            return report

        first = scenario("compare_self_pass", "local")
        risk = scenario("compare_self_risk", "risk", passed_label="compare_self_test_risk_passed")
        # The alignment keeps the two indexes equal after an admission
        # difference: no chain of later differences, and no early stop.
        require(
            risk["unexplained_counts"] == {"admission": 1}
            and risk["unexplained_total"] == 1
            and risk["permitted_counts"] == {"medium_risk_privacy_review": 1}
            and not risk["partial"]
            and risk["alignment_lost_position"] is None
            and not any(risk["distribution"][side]["refused"] for side in SIDES),
            "compare_self_test_risk_fields",
        )
        skewed = scenario(
            "compare_self_skew", "local", skew="baseline_quality_floor", passed_label="compare_self_test_skew_passed"
        )
        # PC-D20: the run stops at the pair after which the indexes can differ.
        require(
            skewed["alignment_lost_position"] is not None and "quality_passed" in skewed["unexplained_counts"],
            "compare_self_test_skew_fields",
        )
        repeat = scenario("compare_self_repeat", "local")
        require(repeat["report_digest"] == first["report_digest"], "compare_self_test_not_deterministic")
    print("PipelineCompareSelfTestOK: scenarios=4")


def run_compare(args, run):
    """`compare --corpus PIN`: exports the pin with its session events,
    checks the export against the pin before any database starts, and runs
    the comparison harness one time, with Cargo's optimized build for the
    two (PC-D21). The check id comes from the pin and from nothing else: a
    pin with a local directory gives `pipeline_comparison_local`, a pin
    without one `pipeline_comparison_hf`.

    A harness that fails and leaves a valid report: the report goes under
    `.local/` with `-partial` or `-failed` in its name, one line names the
    run and the report, and the failure is the first label that the report
    names (the alignment, a refused receipt, an unexplained difference, a
    gate branch with no evidence), or else the step failure. A report write
    that fails does not replace that failure.

    A harness that passes must leave a report that names none of the four
    and that compared the expected number of traces: the limit, or each
    trace of the pin (`comparison_compared_count_mismatch`). A full run must
    also have a pin with all five digests, a current pass result whose
    evidence agrees with the report, and no other result. A partial run
    (`--limit` below the trace count) has no check result
    (`comparison_check_results_unexpected`) and is not evidence: its line
    says `partial=true`. The tree is examined again before the report goes
    under `.local/`. Nothing goes to the lab catalog.

    `compare --self-test`: see `_compare_self_test`."""
    require(args.corpus is not None or args.self_test, "compare_corpus_or_self_test_required")
    require(args.corpus is None or not args.self_test, "compare_corpus_and_self_test_conflict")
    require(args.limit is None or not args.self_test, "compare_self_test_takes_no_option")
    require(args.limit is None or args.limit >= 1, "compare_limit_invalid")
    if args.self_test:
        _compare_self_test(args, run)
        return

    pin_path = Path(args.corpus).resolve()
    pin, local_dir = _compare_pin(pin_path)
    # The declared risks change only the corpus files, so a run that can
    # emit a result needs each digest of the pin.
    pinned = all(pin.get(field) is not None for field in PIN_DIGEST_FIELDS)
    require(pinned or args.limit is not None, "comparison_pin_digest_missing")
    check_id = "pipeline_comparison_local" if local_dir is not None else "pipeline_comparison_hf"
    exported = _compare_input(
        export_compare_corpus(run, pin_path, child_environment({}), name="corpus", local_dir=local_dir, release=True)
    )
    # The number of traces that a pass compares. A limit at or above the
    # trace count does not make a partial run, so the rule for the pin's
    # digests applies before a database starts.
    expected = exported.trace_count if args.limit is None else min(args.limit, exported.trace_count)
    require(pinned or expected < exported.trace_count, "comparison_pin_digest_missing")
    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
        started = time.monotonic()
        report, failure, report_bytes = _compare_once(
            run, environment, exported, check_id, "compare_run", limit=args.limit, release=True
        )
        seconds = int(time.monotonic() - started)

    if failure is not None:
        if report is None:
            raise failure
        label = _compare_failure_label(report)
        try:
            shown = _shown(_write_compare_report(check_id, report, report_bytes, failed=True))
        except OSError:
            # The failure of the run must win: the report stays in the run
            # directory, and no line names a file under `.local/`.
            shown = None
        if shown is not None:
            lost = report["alignment_lost_position"]
            print(
                f"PipelineCompareReport: unexplained={report['unexplained_total']} "
                f"alignment_lost={'none' if lost is None else lost} "
                f"partial={'true' if report['partial'] else 'false'} run={run.run_id} report={shown}"
            )
        raise ToolingError(label) if label is not None else failure
    _require_passing_report(report)
    require(pinned or report["partial"], "comparison_pin_digest_missing")
    require(report["compared_count"] == expected, "comparison_compared_count_mismatch")
    results = load_results(run)
    if report["partial"]:
        require(not results, "comparison_check_results_unexpected")
    else:
        _require_compare_result(run, results, check_id, report, report_bytes)
        require(set(results) == {check_id}, "comparison_check_results_unexpected")
    # Before the report goes under `.local/`: a run that is refused leaves
    # no report there that reads as a pass.
    run.require_code_revision_unchanged()
    shown = _shown(_write_compare_report(check_id, report, report_bytes))
    print(
        f"PipelineCompareOK: traces={report['compared_count']} equal={report['equal_count']} "
        f"permitted={report['permitted_total']} unexplained=0 "
        f"partial={'true' if report['partial'] else 'false'} seconds={seconds} run={run.run_id} report={shown}"
    )


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


def keygen(args, run):
    """Generates a check-signing key and its trusted key by starting the
    ignored test `pipeline_signing_key_write`. It refuses an existing output
    before anything starts; the key is written with mode 0600. It prints the
    key id and no path: the paths go to the test's environment only."""
    require(_KEY_ID.fullmatch(args.key_id) is not None, "signing_key_id_invalid")
    output = Path(args.output).resolve()
    trusted_output = Path(args.trusted_key_output).resolve()
    require(output != trusted_output, "keygen_outputs_must_differ")
    require(not (os.path.lexists(output) or os.path.lexists(trusted_output)), "signing_key_output_exists")
    output.parent.mkdir(parents=True, exist_ok=True)
    trusted_output.parent.mkdir(parents=True, exist_ok=True)
    extra = {
        "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT": str(output),
        "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_KEY_ID": args.key_id,
        "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT": str(trusted_output),
    }
    cargo_test(run, "keygen", INGEST_TEST_ARGS, KEY_WRITER, child_environment(extra), exact=True, ignored=True)

    require(output.is_file() and stat.S_IMODE(output.stat().st_mode) & 0o077 == 0, "signing_key_mode_invalid")
    trusted = _read_json(trusted_output, "trusted_key_invalid")
    require(
        isinstance(trusted, dict)
        and set(trusted) == {"key_id", "public_key_base64url"}
        and trusted["key_id"] == args.key_id
        and isinstance(trusted["public_key_base64url"], str)
        and _PUBLIC_KEY.fullmatch(trusted["public_key_base64url"]) is not None,
        "trusted_key_invalid",
    )
    print(f"PipelineKeygenOK: key_id={args.key_id}")


def revision(args, run):
    """Prints the code revision hash of the working tree: the value `run`
    holds for a command that credits evidence to it (`Run.create`)."""
    print(run.code_revision_hash)


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
    """The seed's fingerprint file: exactly its schema, seven hashes, and
    eight positive counts, two tenants or more among them."""
    value = _read_json(path, "restore_fingerprint_invalid")
    require(
        isinstance(value, dict)
        and set(value) == {"schema", *_RESTORE_FINGERPRINT_HASHES, *_RESTORE_FINGERPRINT_COUNTS}
        and value["schema"] == RESTORE_FINGERPRINT_SCHEMA
        and all(
            isinstance(value[key], str) and _HASH.fullmatch(value[key]) is not None
            for key in _RESTORE_FINGERPRINT_HASHES
        )
        and all(type(value[key]) is int and value[key] > 0 for key in _RESTORE_FINGERPRINT_COUNTS)
        and value["tenant_count"] >= 2,
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
            "rls_tables_checked": seed["rls_table_count"],
            "rls_policy_set_hash": seed["rls_policy_set_hash"],
            "rls_policy_count": seed["rls_policy_count"],
            "rls_flag_set_hash": seed["rls_flag_set_hash"],
            "rls_flag_table_count": seed["rls_flag_table_count"],
            "runtime_privilege_set_hash": seed["runtime_privilege_set_hash"],
            "tenant_fingerprint": seed["tenant_fingerprint"],
            "tenant_count": seed["tenant_count"],
            "audit_events_verified": seed["audit_event_count"],
        },
        "restore_evidence_mismatch",
    )
    return seed


def restore_drill(args, run):
    with Environment(run, postgres_admin_url=args.postgres_admin_url) as environment:
        seed = run_restore_drill(run, environment)
    require(set(load_results(run)) == {RESTORE_CHECK_ID}, "restore_check_results_unexpected")
    run.require_code_revision_unchanged()
    print(
        f"PipelineRestoreOK: database={seed['database_fingerprint']} "
        f"artifacts={seed['artifact_fingerprint']} index={seed['index_entry_set_hash']} "
        f"legs_per_run={seed['completed_settlement_count']} "
        f"credit_events_per_run={seed['completed_credit_event_count']} "
        "pending_runs_resumed=1 duplicate_effects=0"
    )
    print(
        f"PipelineRestoreChecks: rls_tables={seed['rls_table_count']} "
        f"rls_policies={seed['rls_policy_set_hash']} rls_policy_count={seed['rls_policy_count']} "
        f"rls_flags={seed['rls_flag_set_hash']} rls_flag_tables={seed['rls_flag_table_count']} "
        f"runtime_privileges={seed['runtime_privilege_set_hash']} "
        f"runtime_privilege_count={seed['runtime_privilege_count']} "
        f"tenants={seed['tenant_count']} tenant_fingerprint={seed['tenant_fingerprint']} "
        f"audit_events_verified={seed['audit_event_count']}"
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
    replace the original failure: results that no longer load are left out
    (the report then lists none), and a report that cannot be written is
    skipped silently -- `qualify` already removed the previous `.local`
    report when it started, so no older report stands in for this one."""
    if _FAILURE_LABEL.fullmatch(label) is None:
        label = "qualify_failed"
    try:
        try:
            results = load_results(run)
        except Exception:  # noqa: BLE001 -- a result that does not load is left out
            results = {}
        write_report(run, results, inputs, failure=label, local_dir=LOCAL_DIR)
    except Exception:  # noqa: BLE001 -- the original failure must win (see above)
        return


def _shown(path):
    return path.relative_to(ROOT) if path.is_relative_to(ROOT) else path.name


class Signing(NamedTuple):
    """`qualify --signing-key`: the key file (resolved), the id its public key
    is trusted under, and the maximum age each signed result carries."""

    key_path: Path
    key_id: str
    maximum_age_seconds: int


def signing_options(args):
    """The signing options of `qualify`, or `None` without `--signing-key`.
    Every flag is checked before any step runs. The two key flags go
    together (`signing_key_incomplete`); the maximum age is at least one
    second and at most the server's ceiling (`evidence_max_age_invalid`,
    `evidence_max_age_above_ceiling`); the key file must exist
    (`signing_key_unreadable`). The key path appears in no label."""
    require((args.signing_key is None) == (args.signing_key_id is None), "signing_key_incomplete")
    require(args.evidence_max_age_seconds >= 1, "evidence_max_age_invalid")
    require(args.evidence_max_age_seconds <= EVIDENCE_AGE_CEILING_SECONDS, "evidence_max_age_above_ceiling")
    if args.signing_key is None:
        return None
    require(_KEY_ID.fullmatch(args.signing_key_id) is not None, "signing_key_id_invalid")
    key_path = Path(args.signing_key).resolve()
    require(key_path.is_file() and os.access(key_path, os.R_OK), "signing_key_unreadable")
    return Signing(key_path, args.signing_key_id, args.evidence_max_age_seconds)


def _staging_dir(run):
    return run.run_dir / ATTESTATION_STAGING


def _attestations_remain(run):
    """Whether the staging directory, or any attestation file, is in the run
    directory."""
    return os.path.lexists(_staging_dir(run)) or any(run.run_dir.rglob("*.attestation.json"))


def discard_attestations(run):
    """Removes the staging directory and every attestation file of the run's
    results directory: the two places this tool writes them. Called before a
    failed run writes its report, so that a failed run leaves no attestation
    file anywhere in its directory (the server accepts an attestation set
    for the revision it names). Best effort, file by file: one that cannot be
    removed does not stop the others, nor replace the failure. Then it looks
    again, and returns whether nothing is left: a removal that failed without
    a sign would leave a valid attestation behind a report that says there is
    none."""
    shutil.rmtree(_staging_dir(run), ignore_errors=True)
    for path in run.results_dir.glob("*.attestation.json"):
        try:
            path.unlink(missing_ok=True)
        except OSError:
            pass
    return not _attestations_remain(run)


def _fail_closed(run, inputs, label):
    """The start of every failure path of `qualify`, before it re-raises.
    It removes what the run signed, then both copies of this run's report (an
    attested pass report is written before the files are published and the
    archive is made, so a step that fails after it must not leave it behind,
    even when the failed report cannot be written), then writes the failed
    report. If anything signed could not be removed, the terminal gets a line
    `PipelineFailure: check_attestation_discard_failed` and the failure
    report carries `<label>.check_attestation_discard_failed`: the original
    label is still shown, by the caller's re-raise and in the report."""
    clean = discard_attestations(run)
    for path in (LOCAL_DIR / REPORT_NAME, run.run_dir / RUN_REPORT_NAME):
        try:
            path.unlink(missing_ok=True)
        except OSError:
            pass
    if not clean:
        print(f"PipelineFailure: {DISCARD_FAILED_LABEL}", file=sys.stderr)
        label = f"{label}.{DISCARD_FAILED_LABEL}"
    _write_failed_report(run, inputs, label)


def attest_results(run, accepted, signing, cargo_args=INGEST_TEST_ARGS):
    """The signing step: starts the ignored test
    `pipeline_check_attestations_write`, which signs the result file of each
    accepted check (`accepted`: check id to the `CheckResult` that
    `require_current_pass_results` accepted; the test refuses any other result
    file and any accepted check without one) into the staging directory and
    verifies what it wrote, then checks the files it left against the accepted
    results. Runs only for a result set that passed
    `require_current_pass_results`. The files stay in the staging directory
    (`publish_attestations` moves them once every later check has passed). The
    key path goes to the test's environment and nowhere else (the test's own
    output goes to the step's log). Returns the number of attestations."""
    staging = _staging_dir(run)
    discard_attestations(run)
    staging.mkdir(mode=0o700)
    extra = {
        "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR": str(run.results_dir),
        "TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR": str(staging),
        "TRACE_COMMONS_PIPELINE_CHECK_IDS": ",".join(sorted(accepted)),
        "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_PATH": str(signing.key_path),
        "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID": signing.key_id,
        "TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS": str(signing.maximum_age_seconds),
    }
    cargo_test(
        run, "check_attestations", cargo_args, ATTESTATION_WRITER, child_environment(extra), exact=True, ignored=True
    )
    return require_attestations(run, accepted, signing.key_id, signing.maximum_age_seconds, staging)


def _publish_file(source, destination):
    os.replace(source, destination)


def publish_attestations(run, accepted):
    """Moves the staged attestation files to their final names, beside the
    results they sign, and removes the staging directory. The last step of a
    run before it archives anything: it runs only after every check that
    follows the signing step has passed. A file that cannot be moved is
    `check_attestation_publish_failed`; the caller then removes what was
    already moved."""
    staging = _staging_dir(run)
    try:
        for check_id in sorted(accepted):
            name = f"{check_id}.attestation.json"
            _publish_file(staging / name, run.results_dir / name)
        staging.rmdir()
    except OSError as error:
        raise ToolingError("check_attestation_publish_failed") from error


def sign_promoted_results(run, accepted, signing, cargo_args):
    """`promote sign`'s signing step: `attest_results` on `cargo_args`, then
    `publish_attestations`. Anything that fails after signing started
    removes what was signed before the failure is raised, so a failed sign
    leaves no attestation in the run (or says
    `check_attestation_discard_failed` beside the original label)."""
    try:
        attested = attest_results(run, accepted, signing, cargo_args)
        run.require_code_revision_unchanged()
        publish_attestations(run, accepted)
    except BaseException as error:
        if not discard_attestations(run):
            print(f"PipelineFailure: {DISCARD_FAILED_LABEL}", file=sys.stderr)
            if isinstance(error, ToolingError) and not isinstance(error, StepFailed):
                raise ToolingError(f"{error}.{DISCARD_FAILED_LABEL}") from error
        raise
    return attested


def qualify(args, run):
    """Runs every required check (binding checks by exit status, then each
    database check, corpus run, and the restore drill in its own scenario of
    one environment), requires a current pass result for each, and writes
    one bounded report.

    It first removes the previous `.local` report, so an older passing
    report never stays the latest one. A failed run writes its report with
    `status: fail` and the safe label, then raises: a tooling failure under
    its own label, an interrupt (Ctrl-C) as `qualify_interrupted`, anything
    else as `qualify_internal_error`. Only a report that itself cannot be
    written is left out.

    With `--signing-key` (P5-D13), a run whose every required check passed
    then signs each required result (`attest_results`): the attestation files,
    `<check_id>.attestation.json`, are what the server's
    `qualify_bundle_attested` reads, and they name this run's code revision.
    The tree is checked before the signing step and again after it, and the
    files are signed into a staging directory and moved to their final names
    (`publish_attestations`) only once the report is written, so a run that
    fails after signing started removes the staging directory and every
    attestation file before it writes its failed report (`_fail_closed`): a
    failed run leaves none, as its report says (`attested: false`), or, when
    a removal fails, says `check_attestation_discard_failed` beside the
    original label. It also removes this run's own pass report first, so a
    failed run cannot leave that behind when its failed report cannot be
    written. The report of a pass counts them. Flags that are wrong are
    refused before anything runs."""
    signing = signing_options(args)
    inputs = {
        "code_revision_hash": run.code_revision_hash,
        "contract_manifest_digest": None,
        "inventory_digest": None,
        "corpus_runs": [],
    }
    catalog_path = None
    attested = 0
    (LOCAL_DIR / REPORT_NAME).unlink(missing_ok=True)
    try:
        results = _run_required_checks(args, run, inputs)
        accepted = {check_id: results[check_id] for check_id in required_specs()}
        if signing is not None:
            # After `require_current_pass_results` (the end of
            # `_run_required_checks`). A tree edited during the checks is
            # refused before anything is signed.
            run.require_code_revision_unchanged()
            attested = attest_results(run, accepted, signing)
        # Before the report: a tree edited during the run (while the results
        # were signed too) writes a failed report under `code_revision_changed`,
        # never a pass.
        run.require_code_revision_unchanged()
        report_path = write_report(run, results, inputs, local_dir=LOCAL_DIR, attestation_count=attested)
        if signing is not None:
            publish_attestations(run, accepted)
        if args.archive:
            catalog_path = LOCAL_DIR / CATALOG_NAME
            update_catalog(catalog_path, report_path, records=corpus_records(run))
    except ToolingError as error:
        _fail_closed(run, inputs, str(error))
        raise
    except KeyboardInterrupt:
        _fail_closed(run, inputs, "qualify_interrupted")
        raise
    except Exception:
        _fail_closed(run, inputs, "qualify_internal_error")
        raise
    line = f"PipelineQualificationOK: report={_shown(report_path)} checks={len(results)}"
    if attested:
        line += f" attested={attested}"
    if catalog_path is not None:
        line += f" catalog={_shown(catalog_path)}"
    print(line)
    print(
        "PipelineQualificationScope: production_promotion_ready=false blockers="
        + ",".join(SAFE_BLOCKERS)
        + " -- local evidence only, not a production promotion"
    )


def _wrote_anything(run):
    """Whether any file is in the run directory (`Run.create` makes only
    empty directories)."""
    return any(path.is_file() for path in run.run_dir.rglob("*"))


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
    no_logs = not any(run.run_dir.rglob("*.log"))
    mode = getattr(args, "run_dir_mode", None)
    if (
        (getattr(args, "command", None) in EPHEMERAL_COMMANDS and (primary == 0 or no_logs))
        # `promote` subcommands after `init` work in the run `init` made.
        or mode == promote.NEVER_USED
        # `promote init` and `hf-pin record` keep their run unless refused
        # before they wrote anything.
        or (mode == promote.KEEP_UNLESS_REFUSED and primary != 0 and no_logs and not _wrote_anything(run))
    ):
        # `revision` and `keygen` keep nothing: no empty run directory is left
        # behind. A failed step keeps its log, and the failure line names it.
        shutil.rmtree(run.run_dir, ignore_errors=True)
    cleanup = 1 if run.cleanup_failed else 0
    # `qualify` raises `cleanup_failed` itself (its report must say so);
    # the line is printed once.
    if cleanup and primary_label != "cleanup_failed":
        print("PipelineFailure: cleanup_failed", file=sys.stderr)
    return primary if primary != 0 else cleanup


if __name__ == "__main__":
    raise SystemExit(main())
