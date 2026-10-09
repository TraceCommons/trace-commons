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
import os
import re
import secrets
import shutil
import stat
import sys
from pathlib import Path
from typing import NamedTuple, Optional

from pipeline_tooling import envfile, promote
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
# `package` also builds the production package (spec B-D5), from the
# deployment's env file; `run` never serves it as a built-in bundle.
PACKAGE_BUNDLES = (*BUNDLES, "production")

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
    package_parser.add_argument("--bundle", choices=PACKAGE_BUNDLES, required=True)
    package_parser.add_argument(
        "--env-file",
        dest="env_file",
        default=None,
        help="The deployment's env file (--bundle production only); only its descriptor and gate variables are read.",
    )
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


def run_package(args, run):
    require((args.signing_key is None) == (args.key_id is None), "package_signing_key_and_key_id_required")
    require(args.key_id is None or _KEY_ID.fullmatch(args.key_id) is not None, "package_key_id_invalid")
    output = Path(args.output).resolve()
    key_output = Path(args.public_key_output).resolve()
    require(output != key_output, "package_outputs_must_differ")
    production = args.bundle == "production"
    require(not production or args.env_file is not None, "package_env_file_required")
    require(production or args.env_file is None, "package_env_file_unexpected")
    extra = {
        "TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE": args.bundle,
        "TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT": str(output),
        "TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT": str(key_output),
    }
    if production:
        # Spec B-D5: the production package from the deployment's own
        # descriptor and gate variables, offline; nothing else of the file.
        extra.update(envfile.allowlisted(envfile.read_env_file(args.env_file), envfile.PACKAGE_VARIABLES))
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
