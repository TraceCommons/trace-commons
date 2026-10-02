"""The qualification report `pipeline.py qualify` writes
(`trace_commons.pipeline_qualification_report.v1`).

Ports the report shape of `ef97a459:scripts/operator/run-pipeline-
qualification.sh` lines 50-176, with executed results in place of the
port's declared drills:

- `inputs`: this run's code revision hash, one entry for each corpus run
  (its check id, bundle id, package hash, configuration and dependency
  digests, corpus digests, and report digest), the `inventory_digest` of the
  deployment inventory the run wrote (ruling T11-3), and the SHA-256 of the
  contract test manifest file's bytes (ruling T11-2; its test ids are not
  validated, P4-D14).
- `checks`: every check result of this run that validates, with its bounded
  evidence, so the report verifies on its own.
- `safe_blockers`: the local blockers, plus any blocker a check result
  carries (a pass result's blockers are promotion blockers).
- `status` (`pass`, or `fail` with the safe `failure` label), and
  `evidence_hash`, the SHA-256 of the canonical `{"inputs", "checks"}`.
- `attested` and `attestation_count`: whether `qualify --signing-key` signed
  the results (P5-D13), and how many attestation files it left (one for each
  required check of a pass report). Neither is under `evidence_hash`: the
  attestations are separate files, signed and verified on their own. A failed
  report is never attested, and a failed run leaves no attestation file at all
  (`pipeline.qualify` stages the files and removes them on any failure). The
  schema stays `v1`. A `v1` report without the two keys validates and reads as
  unsigned (`report_attestation`: `attested: false`, `attestation_count: 0`);
  a report that has one of the two must have both. That a report without them
  validates is not a promise that every report of `main`'s tool does: a FAIL
  report from `main`'s tool validates; a PASS report from `main`'s tool does
  not, because its evidence names more than one package (the minimal corpus
  check and each mechanics check carry a package of their own, and a run names
  many), which `_require_one_package` refuses: such a result set could never
  back a promotion.

Not carried from the port: `base_revision_hash` (the code revision hash
already binds the tree) and the fixed `acceptance_layers` list, which no
executed check backs. `production_promotion_ready` is always false: every
check runs on local reference or synthetic adapters, which `safe_blockers`
names, and promotion is `evaluate_promotion`'s decision over
`PROMOTION_REQUIRED_CHECKS`, three of which no local or CI run can pass.
"""

from __future__ import annotations

import json
import re
from datetime import datetime, timezone
from pathlib import Path

from .checks import REQUIRED_CHECK_IDS, REQUIRED_CORPUS_CHECK_IDS, required_specs
from .environment import ROOT
from .errors import ToolingError, require
from .files import atomic_write, sha256_digest
from .results import canonical, validate_evidence

REPORT_SCHEMA = "trace_commons.pipeline_qualification_report.v1"
# The latest report, under `.local/`; the run directory keeps its own copy.
REPORT_NAME = "pipeline-qualification-report.json"
RUN_REPORT_NAME = "qualification-report.json"

# Why a passing local qualification is still not production evidence: the
# corpus harness's reference and synthetic adapters and static tokens, the
# restore drill's local filesystem copy, and the HF corpus coming from the
# local fixture pin with no network canary (ruling HF-1).
SAFE_BLOCKERS = (
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
    "filesystem_restore_local_only",
    "hf_network_canary_not_run",
)

_REPORT_KEYS = frozenset(
    {
        "schema",
        "generated_at",
        "scope",
        "status",
        "failure",
        "production_promotion_ready",
        "external_payout_enabled",
        "safe_blockers",
        "run_id",
        "inputs",
        "checks",
        "evidence_hash",
    }
)
# Written by this tool, optional on read (see the module docstring).
_ATTESTATION_KEYS = frozenset({"attested", "attestation_count"})
_INPUT_HASHES = ("code_revision_hash", "contract_manifest_digest", "inventory_digest")
_CORPUS_RUN_HASHES = ("bundle_id", "package_hash", "configuration_digest", "dependency_digest", "report_digest")
_CORPUS_RUN_KEYS = frozenset({"check_id", "corpus_digests", "fixture_count", *_CORPUS_RUN_HASHES})
_CHECK_DIGESTS = ("package_hash", "configuration_digest", "dependency_digest")
_CHECK_KEYS = frozenset(
    {"check_id", "status", "observed_at", "evidence_hash", "safe_blockers", "evidence", *_CHECK_DIGESTS}
)
_HASH = re.compile(r"sha256:[0-9a-f]{64}\Z")
_LABEL = re.compile(r"[a-z0-9_]{1,64}\Z")
# A `ToolingError` label, which may carry a check id after a colon.
_FAILURE = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")


def _is_hash(value):
    return isinstance(value, str) and _HASH.fullmatch(value) is not None


def _is_label(value):
    return isinstance(value, str) and _LABEL.fullmatch(value) is not None


def _iso(when):
    return when.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def corpus_run_input(report):
    """The `inputs.corpus_runs` entry for one validated corpus report. It
    names the package the run served, so the minimal run's entry carries
    its test bundle's digests even though that check's result names none
    (P5-D15)."""
    return {
        "check_id": report["check_id"],
        "bundle_id": report["bundle_id"],
        "package_hash": report["package_hash"],
        "configuration_digest": report["configuration_digest"],
        "dependency_digest": report["dependency_digest"],
        "corpus_digests": [section["corpus_digest"] for section in report["partitions"]],
        "report_digest": report["report_digest"],
        "fixture_count": report["fixture_count"],
    }


def _evidence(run, result):
    """The result's evidence file, which must still pass the evidence
    validator and hash to the result's `evidence_hash`."""
    try:
        evidence = json.loads((run.results_dir / f"{result.check_id}.evidence.json").read_bytes())
    except (OSError, ValueError) as error:
        raise ToolingError("check_evidence_malformed") from error
    validate_evidence(evidence)
    require(sha256_digest(canonical(evidence)) == result.evidence_hash, "check_evidence_hash_mismatch")
    return evidence


def check_entries(run, results, *, strict=True):
    """One `checks` entry for each result of this run, in check id order.
    `strict` raises on a result from another run or code revision, or whose
    evidence no longer validates; otherwise (a report for a run that already
    failed) that result is left out, so a report never lists a result it
    does not own."""
    entries = []
    for check_id in sorted(results):
        result = results[check_id]
        try:
            require(result.run_id == run.run_id, "check_result_foreign_run")
            require(result.code_revision_hash == run.code_revision_hash, "check_result_foreign_revision")
            evidence = _evidence(run, result)
        except ToolingError:
            if strict:
                raise
            continue
        entries.append(
            {
                "check_id": check_id,
                "status": result.status,
                "package_hash": result.package_hash,
                "configuration_digest": result.configuration_digest,
                "dependency_digest": result.dependency_digest,
                "observed_at": _iso(result.observed_at),
                "evidence_hash": result.evidence_hash,
                "safe_blockers": list(result.safe_blockers),
                "evidence": evidence,
            }
        )
    return entries


def _report_blockers(checks):
    """The local blockers, then any other blocker a check result carries (a
    pass result's blockers are promotion blockers), each once."""
    extra = sorted({label for item in checks for label in item["safe_blockers"]}.difference(SAFE_BLOCKERS))
    return [*SAFE_BLOCKERS, *extra]


def build_report(run, checks, inputs, *, failure=None, attestation_count=0):
    """The report value. `inputs` holds `code_revision_hash`,
    `contract_manifest_digest`, `inventory_digest` (each `None` when a failed
    run never reached it), and `corpus_runs`. `attestation_count` is how many
    attestation files the signing step wrote (0 when it did not run)."""
    inputs = {
        "code_revision_hash": inputs.get("code_revision_hash"),
        "contract_manifest_digest": inputs.get("contract_manifest_digest"),
        "inventory_digest": inputs.get("inventory_digest"),
        "corpus_runs": sorted(inputs.get("corpus_runs", ()), key=lambda item: item["check_id"]),
    }
    return {
        "schema": REPORT_SCHEMA,
        "generated_at": _iso(datetime.now(timezone.utc)),
        "scope": "local_test",
        "status": "pass" if failure is None else "fail",
        "failure": failure,
        "production_promotion_ready": False,
        "external_payout_enabled": False,
        "safe_blockers": _report_blockers(checks),
        "run_id": run.run_id,
        "attested": attestation_count > 0,
        "attestation_count": attestation_count,
        "inputs": inputs,
        "checks": checks,
        "evidence_hash": sha256_digest(canonical({"inputs": inputs, "checks": checks})),
    }


def validate_qualification_report(report):
    """Refuses a report that is not exactly this shape, carries anything but
    labels, hashes, counts, booleans, and ISO times, claims promotion, drops
    a local blocker, does not hash to its `evidence_hash`, or (when it says
    `pass`) lacks a required check or input. A missing field or a field of
    the wrong type fails with `qualification_report_invalid`, never a
    traceback."""
    try:
        _validate(report)
    except (KeyError, TypeError, AttributeError) as error:
        raise ToolingError("qualification_report_invalid") from error


def report_attestation(report):
    """`(attested, attestation_count)` of a report. A report that has neither
    key reads as `(False, 0)`: one written before the keys existed, or by
    `main`'s tool, is unsigned, so that reading is true."""
    return report.get("attested", False), report.get("attestation_count", 0)


def _validate(report):
    require(isinstance(report, dict), "qualification_report_invalid")
    present = set(report) & _ATTESTATION_KEYS
    # Either both attestation keys or neither: one alone is refused.
    require(
        present in (set(), _ATTESTATION_KEYS)
        and set(report) - present == _REPORT_KEYS
        and report["schema"] == REPORT_SCHEMA,
        "qualification_report_invalid",
    )
    validate_evidence(report)
    require(
        report["scope"] == "local_test"
        and report["production_promotion_ready"] is False
        and report["external_payout_enabled"] is False,
        "qualification_report_scope_invalid",
    )
    require(set(SAFE_BLOCKERS).issubset(report["safe_blockers"]), "qualification_report_blockers_missing")
    passed = report["status"] == "pass"
    require(report["status"] in ("pass", "fail"), "qualification_report_invalid")
    failure = report["failure"]
    require(
        (failure is None) if passed else (isinstance(failure, str) and _FAILURE.fullmatch(failure) is not None),
        "qualification_report_invalid",
    )
    require(_is_label(report["run_id"]), "qualification_report_invalid")
    attested, count = report_attestation(report)
    require(
        type(count) is int and count >= 0 and attested is (count > 0) and (passed or count == 0),
        "qualification_report_invalid",
    )

    inputs = report["inputs"]
    require(isinstance(inputs, dict) and set(inputs) == {*_INPUT_HASHES, "corpus_runs"}, "qualification_report_invalid")
    for key in _INPUT_HASHES:
        require(inputs[key] is None or _is_hash(inputs[key]), "qualification_report_invalid")
    runs = inputs["corpus_runs"]
    require(isinstance(runs, list), "qualification_report_invalid")
    for item in runs:
        require(isinstance(item, dict) and set(item) == _CORPUS_RUN_KEYS, "qualification_report_invalid")
        require(_is_label(item["check_id"]), "qualification_report_invalid")
        require(all(_is_hash(item[key]) for key in _CORPUS_RUN_HASHES), "qualification_report_invalid")
        require(
            isinstance(item["corpus_digests"], list)
            and item["corpus_digests"]
            and all(_is_hash(digest) for digest in item["corpus_digests"]),
            "qualification_report_invalid",
        )
        require(type(item["fixture_count"]) is int and item["fixture_count"] > 0, "qualification_report_invalid")
    run_ids = [item["check_id"] for item in runs]
    require(run_ids == sorted(set(run_ids)), "qualification_report_invalid")

    checks = report["checks"]
    require(isinstance(checks, list), "qualification_report_invalid")
    for item in checks:
        require(isinstance(item, dict) and set(item) == _CHECK_KEYS, "qualification_report_invalid")
        require(_is_label(item["check_id"]), "qualification_report_invalid")
        require(item["status"] in ("pass", "fail", "blocked"), "qualification_report_invalid")
        require(all(item[key] is None or _is_hash(item[key]) for key in _CHECK_DIGESTS), "qualification_report_invalid")
        require(
            isinstance(item["safe_blockers"], list) and all(_is_label(label) for label in item["safe_blockers"]),
            "qualification_report_invalid",
        )
        require(
            sha256_digest(canonical(item["evidence"])) == item["evidence_hash"],
            "qualification_report_digest_mismatch",
        )
    check_ids = [item["check_id"] for item in checks]
    require(check_ids == sorted(set(check_ids)), "qualification_report_invalid")
    require(
        all(set(item["safe_blockers"]).issubset(report["safe_blockers"]) for item in checks),
        "qualification_report_blockers_missing",
    )
    require(
        sha256_digest(canonical({"inputs": inputs, "checks": checks})) == report["evidence_hash"],
        "qualification_report_digest_mismatch",
    )

    if passed:
        status = {item["check_id"]: item["status"] for item in checks}
        require(
            all(inputs[key] is not None for key in _INPUT_HASHES)
            and set(run_ids) == set(REQUIRED_CORPUS_CHECK_IDS)
            and all(status.get(check_id) == "pass" for check_id in REQUIRED_CHECK_IDS),
            "qualification_report_incomplete",
        )
        _require_one_package(checks)
        # An attested pass report has one attestation for each check it lists
        # (a signed run refuses a result file that is not a required check).
        require(not attested or count == len(checks), "qualification_report_invalid")


def _require_one_package(checks):
    """A pass report names exactly one package (P5-D15), the rules of the
    results themselves (`results.require_current_pass_results`,
    `results.require_one_package`) applied to the report's `checks`, which
    `catalog.py` also reads from disk: a check whose spec asks for digests
    carries all three, any other required check carries none, and the
    digests present are one set."""
    specs = required_specs()
    packages = set()
    for item in checks:
        check_id = item["check_id"]
        digests = tuple(item[key] for key in _CHECK_DIGESTS)
        spec = specs.get(check_id)
        if spec is not None and spec.digests_required:
            require(None not in digests, f"check_result_digest_missing:{check_id}")
        elif spec is not None:
            require(digests == (None, None, None), f"pipeline_check_digests_unexpected:{check_id}")
        if digests != (None, None, None):
            packages.add(digests)
    require(len(packages) <= 1, "qualification_evidence_mixed_package")


def write_report(run, results, inputs, *, failure=None, local_dir=None, attestation_count=0):
    """Builds and validates the report, writes it to the run directory and
    to `<local_dir>/pipeline-qualification-report.json` (the latest report;
    `local_dir` defaults to `.local/`), and returns the latter path. With a
    `failure` label the report says `status: fail`, and a result whose
    evidence no longer validates is left out instead of raising.
    `attestation_count` is how many attestations the run signed (a failed
    report has none)."""
    checks = check_entries(run, results, strict=failure is None)
    report = build_report(run, checks, inputs, failure=failure, attestation_count=attestation_count)
    validate_qualification_report(report)
    data = json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False).encode() + b"\n"
    atomic_write(run.run_dir / RUN_REPORT_NAME, data)
    local_path = Path(local_dir if local_dir is not None else ROOT / ".local") / REPORT_NAME
    atomic_write(local_path, data)
    return local_path
