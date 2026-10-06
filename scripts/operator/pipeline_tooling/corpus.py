"""HF corpus loading and export helpers for pipeline_tooling.

`load_direct_corpus` ports `validate_corpus` from
`ef97a459:scripts/operator/lab/lab.py` lines 102-119: it is the shape check
every `trace_commons.pipeline_corpus.v1` file (an HF export, or a hand-
written corpus fixture) must pass before it runs through the shared ingest
app.

`load_pin` and `export_hf_corpus` port the pin-reading and manifest-checking
half of `ef97a459:scripts/operator/run-pipeline-hf-qualification.sh`: the
argument list built at lines 41-60, and the remote digest check at lines
101-149 (`if mode == "remote": ... require(source[field] == pin[field])`).
`export_hf_corpus` drives the Task 4 export binary
(`trace-commons-pipeline-corpus-export`) through `run_child`, so a failing
export raises the ordinary `StepFailed` (step label, exit code, protected
log path -- never the child's output).

The four digest fields the shell script's remote branch compares
(`source_digest`, `order_digest`, `bootstrap_corpus_digest`,
`holdout_corpus_digest`) are checked here whenever the pin names them --
`source_digest` and `order_digest` always (`load_pin` requires both), the two
corpus digests only when the pin sets them (a draft pin may not have run the
export yet). `configuration_digest` is not compared: it is derived from the
same pin fields the CLI arguments already carry, so a mismatch there is a
binary or pin defect the corpus-digest checks would already have caught, not
a separate signal to check.

`safe_report_value`, `validate_report`, and `markdown` port
`ef97a459:scripts/operator/lab/lab.py` lines 122-216 and 238-251, adapted to
the `trace_commons.pipeline_corpus_report.v1` report the ignored Rust harness
`pipeline_corpus_pg_tests::pipeline_corpus_run` writes: one section per corpus
partition (`corpus`, or `bootstrap` then `holdout` for an HF pin), run ids as
SHA-256 hashes only, the report digest over its canonical JSON, and each
fixture's `mismatches` recomputed here from its observed and expected fields.
"""

from __future__ import annotations

import json
import re
import uuid
from pathlib import Path

from .environment import ROOT, run_child
from .errors import ToolingError, require
from .files import sha256_digest
from .results import canonical

CORPUS_SCHEMA = "trace_commons.pipeline_corpus.v1"
PIN_SCHEMA = "trace_commons.pipeline_hf_corpus_pin.v1"
REPORT_SCHEMA = "trace_commons.pipeline_corpus_report.v1"

# The corpus `pipeline.py run` uses when `--corpus` is not given: `main`'s
# #971 text, unchanged (P4-D16 derives the expectations it leaves out).
DEFAULT_CORPUS = ROOT / "docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json"

PHASES = ("admission", "review", "score", "settle")

# The check ids the harness may emit (`pipeline_corpus_pg_tests`'s
# `CORPUS_CHECK_IDS`). `pipeline_http_corpus_package` is not a required check
# (controller ruling PF-1).
CORPUS_CHECK_IDS = frozenset(
    {
        "pipeline_http_corpus_minimal",
        "pipeline_http_corpus_compatibility",
        "pipeline_http_corpus_hf_local",
        "pipeline_http_corpus_package",
    }
)

# Port `lab.py` `BLOCKERS`: every corpus report is local test evidence.
LOCAL_BLOCKERS = (
    "local_test_only",
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
)

_REPORT_HASH = re.compile(r"sha256:[a-f0-9]{64}\Z")
_REPORT_LABEL = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")
_REPORT_UUID = re.compile(r"[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}\Z")
_VOLATILE = frozenset({"duration_ms", "time_in_phase_ms", "next_attempt_at"})
_PRIVATE_REPORT_FIELDS = frozenset(
    {"input", "text", "trace_text", "secret", "secret_probe", "token", "account_id", "email"}
)
# Secret-shaped values: provider token prefixes, and the `token-...` shape
# of a static bearer credential (the harness's own tokens have it).
_SECRET_PREFIXES = ("ghp_", "github_pat_", "sk-", "token-")
# A run the worker finished: `complete`, or `rejected` (Admission or Review
# rejected it; its run is complete too).
_TERMINAL_STATES = ("complete", "rejected")
# The observed fields compared with their `expected_` counterpart, in the
# order `pipeline_corpus_pg_tests::fixture_mismatches` names them.
_COMPARED_STATES = ("consent_state", "privacy_state", "scoring_state", "settlement_state")
_REQUIRED_FLAGS = ("replay_same_run", "changed_content_refused", "tenant_isolation")

# Every HF download made by `export_hf_corpus` stays inside the worktree:
# without an explicit `--cache-dir`, hf-hub falls back to `$HF_HOME` or
# `~/.cache/huggingface`. One cache is shared across runs (git-ignored, like
# the rest of `.local/`) and created on first use.
HF_CACHE_DIR = ROOT / ".local" / "pipeline" / "hf-cache"

_LABEL = re.compile(r"[a-z0-9_]{1,64}\Z")

# Exactly the fields `run-pipeline-hf-qualification.sh` reads out of the pin
# (its `PINNED` array plus `source_digest`/`order_digest`). The two corpus
# digests and `configuration_digest` are not required here: a pin drafted
# before its first export cannot know them yet.
_PIN_REQUIRED_FIELDS = (
    "repository",
    "revision",
    "split",
    "translator",
    "bootstrap_count",
    "holdout_count",
    "min_words",
    "max_words",
    "expected_instrument_count",
    "source_digest",
    "order_digest",
)

_MANIFEST_DIGEST_FIELDS = (
    "source_digest",
    "order_digest",
    "bootstrap_corpus_digest",
    "holdout_corpus_digest",
)

MANIFEST_SCHEMA = "trace_commons.pipeline_hf_corpus_manifest.v1"
# Exactly the keys `trace-commons-pipeline-corpus-export` writes into
# `source-manifest.json`, and into its `source` object.
_MANIFEST_KEYS = frozenset(
    {
        "schema",
        "source",
        *_MANIFEST_DIGEST_FIELDS,
        "configuration_digest",
        "sample_count",
        "bootstrap_count",
        "holdout_count",
        "contains_raw_trace_text",
        "contains_contributor_identity",
    }
)
_MANIFEST_SOURCE_LABELS = ("revision", "split", "translator")
_MANIFEST_SOURCE_COUNTS = (
    "bootstrap_count",
    "holdout_count",
    "min_words",
    "max_words",
    "expected_instrument_count",
)
_MANIFEST_COUNTS = ("sample_count", "bootstrap_count", "holdout_count")
# A public dataset name, `owner/name`.
_REPOSITORY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,95}/[A-Za-z0-9][A-Za-z0-9_.-]{0,95}\Z")


def load_direct_corpus(path, expected_digest=None):
    """Loads and validates a `trace_commons.pipeline_corpus.v1` file. Returns
    `(corpus, digest)` -- the parsed JSON and the `sha256:` digest of its raw
    bytes (ports `validate_corpus`, lab.py lines 102-119)."""
    data = Path(path).read_bytes()
    corpus = json.loads(data)
    require(corpus.get("schema") == CORPUS_SCHEMA, "unsupported_corpus_schema")
    fixtures = corpus.get("fixtures")
    require(bool(fixtures), "empty_corpus")
    seen = {field: set() for field in ("label", "trace_id", "submission_id")}
    for fixture in fixtures:
        label = fixture.get("label")
        require(isinstance(label, str) and _LABEL.fullmatch(label) is not None, "unsafe_fixture_label")
        for field, values in seen.items():
            value = fixture.get(field)
            require(value is not None, "missing_corpus_identity")
            require(value not in values, "duplicate_corpus_identity")
            values.add(value)
        uuid.UUID(fixture["trace_id"])
        uuid.UUID(fixture["submission_id"])
        require(bool(fixture.get("secret_probe")), "empty_secret_probe")
    actual = sha256_digest(data)
    require(expected_digest is None or actual == expected_digest, "corpus_digest_mismatch")
    return corpus, actual


def load_pin(path):
    """Loads a `trace_commons.pipeline_hf_corpus_pin.v1` descriptor and
    refuses one missing a field the port script reads out of it."""
    pin = json.loads(Path(path).read_text())
    require(isinstance(pin, dict) and pin.get("schema") == PIN_SCHEMA, "unsupported_pin_schema")
    for field in _PIN_REQUIRED_FIELDS:
        require(field in pin, "pin_missing_field")
    return pin


def export_hf_corpus(run, pin_path, env, *, local_dir=None):
    """Runs the Task 4 export binary with the settings from `pin_path`,
    writing into `run.run_dir / "hf"`, then checks the manifest against the
    pin's digests. Returns `[bootstrap-corpus.json, holdout-corpus.json]`.

    `local_dir`, when given, is passed through as `--local-jsonl-dir`
    (a local fixture run, as CI uses); the pin's own `source_digest` and
    `order_digest` are used as `--expected-*-digest` either way, exactly as
    the port script does for both its local and remote modes. `--cache-dir`
    is always `HF_CACHE_DIR` (one code path for both modes; harmless when
    `local_dir` is set, since no download happens then), so a real network
    export never writes outside the worktree."""
    pin = load_pin(pin_path)
    output_dir = run.run_dir / "hf"
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "trace-commons-server",
        "--bin",
        "trace-commons-pipeline-corpus-export",
        "--",
        "--repository",
        str(pin["repository"]),
        "--revision",
        str(pin["revision"]),
        "--split",
        str(pin["split"]),
        "--translator",
        str(pin["translator"]),
        "--output-dir",
        str(output_dir),
        "--bootstrap-count",
        str(pin["bootstrap_count"]),
        "--holdout-count",
        str(pin["holdout_count"]),
        "--min-words",
        str(pin["min_words"]),
        "--max-words",
        str(pin["max_words"]),
        "--expected-instrument-count",
        str(pin["expected_instrument_count"]),
        "--expected-source-digest",
        str(pin["source_digest"]),
        "--expected-order-digest",
        str(pin["order_digest"]),
    ]
    HF_CACHE_DIR.mkdir(parents=True, exist_ok=True)
    command += ["--cache-dir", str(HF_CACHE_DIR)]
    if local_dir is not None:
        command += ["--local-jsonl-dir", str(local_dir)]

    run_child(run, "hf_corpus_export", command, env)

    manifest = json.loads((output_dir / "source-manifest.json").read_text())
    for field in _MANIFEST_DIGEST_FIELDS:
        expected = pin.get(field)
        if expected is not None:
            require(manifest.get(field) == expected, f"hf_{field}_mismatch")
    require(manifest.get("contains_raw_trace_text") is False, "hf_manifest_contains_raw_trace_text")

    return [output_dir / "bootstrap-corpus.json", output_dir / "holdout-corpus.json"]


def validate_hf_manifest(manifest):
    """The export binary's `source-manifest.json`, exactly: its fixed keys,
    `sha256:` digests, whole-number counts, label-shaped source settings,
    the dataset's `owner/name`, and both privacy flags false. The dataset
    name is the one value with a `/`, which `results.validate_evidence`'s
    labels do not allow, so the catalog archives a manifest only after this
    check (and never after a looser one)."""
    require(
        isinstance(manifest, dict) and set(manifest) == _MANIFEST_KEYS and manifest["schema"] == MANIFEST_SCHEMA,
        "hf_manifest_invalid",
    )
    source = manifest["source"]
    require(
        isinstance(source, dict)
        and set(source) == {"repository", *_MANIFEST_SOURCE_LABELS, *_MANIFEST_SOURCE_COUNTS},
        "hf_manifest_invalid",
    )
    require(
        isinstance(source["repository"], str) and _REPOSITORY.fullmatch(source["repository"]) is not None,
        "hf_manifest_invalid",
    )
    for key in _MANIFEST_SOURCE_LABELS:
        value = source[key]
        require(isinstance(value, str) and _REPORT_LABEL.fullmatch(value) is not None, "hf_manifest_invalid")
    for value in [source[key] for key in _MANIFEST_SOURCE_COUNTS] + [manifest[key] for key in _MANIFEST_COUNTS]:
        require(type(value) is int and value >= 0, "hf_manifest_invalid")
    for key in (*_MANIFEST_DIGEST_FIELDS, "configuration_digest"):
        value = manifest[key]
        require(isinstance(value, str) and _REPORT_HASH.fullmatch(value) is not None, "hf_manifest_invalid")
    require(
        manifest["contains_raw_trace_text"] is False and manifest["contains_contributor_identity"] is False,
        "hf_manifest_invalid",
    )


def safe_report_value(value):
    """Ports `lab.py`'s `safe_report_value`: only structured values, hashes,
    and labels. A raw UUID is replaced with its hash and a volatile field is
    dropped, so `validate_report` can require that a report already equals
    its safe form. Floats are refused (the report has none)."""
    if isinstance(value, dict):
        require(all(isinstance(key, str) and _REPORT_LABEL.fullmatch(key) for key in value), "unsafe_report_field")
        require(not _PRIVATE_REPORT_FIELDS.intersection(value), "private_report_field")
        return {key: safe_report_value(item) for key, item in value.items() if key not in _VOLATILE}
    if isinstance(value, list):
        return [safe_report_value(item) for item in value]
    if isinstance(value, str):
        if _REPORT_UUID.fullmatch(value):
            return sha256_digest(value.encode())
        require(_REPORT_LABEL.fullmatch(value) is not None, "unsafe_report_value")
        require(not value.startswith(_SECRET_PREFIXES), "unsafe_report_value")
        return value
    require(value is None or isinstance(value, (bool, int)), "unsafe_report_value")
    return value


def evidence_digests(report):
    """`(corpus_digest, input_digest)`: the two digests the corpus harness's
    evidence carries beside its counts, for an attestation to sign (P5-D14),
    recomputed here from the validated `report`.

    `corpus_digest` is the digest of the normalized direct corpus the run
    loaded: the one partition's own digest (the SHA-256 of its file), or, for
    the two partitions of an HF pin, the canonical hash of the list of both
    digests in order. `input_digest` is the canonical hash of the list of
    every fixture's `request_content_hash`, in the order the run posted them.
    `corpus_evidence_digests` in `pipeline_corpus_pg_tests.rs` computes the
    same two values (each pins the same vectors)."""
    sections = report["partitions"]
    corpus_digests = [section["corpus_digest"] for section in sections]
    requests = [item["request_content_hash"] for section in sections for item in section["fixtures"]]
    corpus_digest = corpus_digests[0] if len(corpus_digests) == 1 else sha256_digest(canonical(corpus_digests))
    return corpus_digest, sha256_digest(canonical(requests))


def fixture_mismatches(item):
    """The labels of every expectation `item` misses, in the harness's
    order: the same rule `pipeline_corpus_pg_tests::fixture_mismatches`
    applies when it fills `mismatches`."""
    found = []
    if item["state"] not in _TERMINAL_STATES:
        found.append("run_not_terminal")
    if item["admission_decision"] != item["expected_admission_decision"]:
        found.append("admission_decision")
    if item["phase_count"] != item["expected_outcome_count"]:
        found.append("outcome_count")
    for field in _COMPARED_STATES:
        if item[field] != item[f"expected_{field}"]:
            found.append(field)
    if item["instrument_count"] != item["expected_instrument_count"]:
        found.append("instrument_count")
    if item["instrument_states"] != item["expected_instrument_states"]:
        found.append("instrument_states")
    for flag in _REQUIRED_FLAGS:
        if item[flag] is not True:
            found.append(flag)
    return found


def validate_report(report):
    """Ports `lab.py`'s `validate_report` to the v1 corpus report. A report
    with recorded fixture failures is valid; one whose failures do not match
    its own fields, whose digests do not verify, or that carries anything
    but labels, hashes, counts, and booleans is not. A report missing a
    field or holding one of the wrong type fails with the label
    `corpus_report_malformed`, never a traceback."""
    try:
        _validate_report(report)
    except (KeyError, TypeError, AttributeError, IndexError) as error:
        raise ToolingError("corpus_report_malformed") from error


def _validate_report(report):
    require(isinstance(report, dict) and report.get("schema") == REPORT_SCHEMA, "unsupported_report_schema")
    require(report.get("scope") == "local_test" and report.get("production_ready") is False, "invalid_report_scope")
    require(report.get("external_payout_enabled") is False, "payout_enabled")
    require(set(LOCAL_BLOCKERS).issubset(report.get("safe_blockers") or ()), "missing_local_blockers")
    require(safe_report_value(report) == report, "unsafe_report")
    require(report.get("check_id") in CORPUS_CHECK_IDS, "invalid_report_check_id")
    for field in ("bundle_id", "package_hash", "configuration_digest", "dependency_digest", "report_digest"):
        require(isinstance(report.get(field), str) and _REPORT_HASH.fullmatch(report[field]), "invalid_report_hash")
    unsigned = {key: value for key, value in report.items() if key != "report_digest"}
    require(sha256_digest(canonical(unsigned)) == report["report_digest"], "report_digest_mismatch")
    manifest = report.get("policy_manifest")
    require(
        isinstance(manifest, dict) and all(isinstance(manifest.get(phase), dict) for phase in PHASES),
        "report_manifest_invalid",
    )
    configuration = {phase: manifest[phase].get("configuration_hash") for phase in PHASES}
    require(sha256_digest(canonical(configuration)) == report["configuration_digest"], "configuration_digest_mismatch")

    sections = report.get("partitions")
    require(isinstance(sections, list), "report_partitions_invalid")
    require(
        [section.get("partition") for section in sections] in (["corpus"], ["bootstrap", "holdout"]),
        "report_partitions_invalid",
    )
    every = []
    for section in sections:
        require(section.get("bundle_id") == report["bundle_id"], "bundle_mismatch")
        require(_REPORT_HASH.fullmatch(section.get("corpus_digest") or "") is not None, "invalid_report_hash")
        fixtures = section.get("fixtures")
        require(isinstance(fixtures, list), "fixture_count_mismatch")
        require(section.get("fixture_order") == [item.get("label") for item in fixtures], "fixture_order_mismatch")
        require(len(fixtures) == section.get("expected_fixture_count") and len(fixtures) > 0, "fixture_count_mismatch")
        require(
            sum(item["state"] in _TERMINAL_STATES for item in fixtures) == section.get("completed_fixture_count"),
            "completion_count_mismatch",
        )
        for item in fixtures:
            require(
                item["instrument_states"]
                == {instrument["instrument_id"]: instrument["internal_settlement_state"] for instrument in item["instruments"]},
                "instrument_states_mismatch",
            )
            require(item.get("mismatches") == fixture_mismatches(item), "qualification_mismatch_not_failed")
        require(
            sum(bool(item["mismatches"]) for item in fixtures) == section.get("failure_count"),
            "failure_count_invalid",
        )
        every.extend(fixtures)
    require(len({item["label"] for item in every}) == len(every), "duplicate_fixture_label")
    require(report.get("fixture_count") == len(every), "fixture_count_mismatch")
    require(
        report.get("completed_fixture_count") == sum(section["completed_fixture_count"] for section in sections),
        "completion_count_mismatch",
    )
    require(
        report.get("failure_count") == sum(section["failure_count"] for section in sections),
        "failure_count_invalid",
    )
    require(
        report.get("replay_same_run_count") == sum(item["replay_same_run"] is True for item in every),
        "replay_count_mismatch",
    )
    require(
        report.get("changed_content_refused_count") == sum(item["changed_content_refused"] is True for item in every),
        "changed_content_count_mismatch",
    )
    require(
        report.get("tenant_isolation") is all(item["tenant_isolation"] is True for item in every),
        "tenant_isolation_mismatch",
    )


def markdown(report):
    """Ports `lab.py`'s `markdown` to the v1 report: the bundle, the phase
    policies, and one fixture table for each partition, with its
    mismatches."""
    lines = [
        "# Pipeline corpus report",
        "",
        f"Check: `{report['check_id']}`",
        "",
        f"Bundle: `{report['bundle_id']}`",
        "",
        f"Package: `{report['package_hash']}`",
        "",
        f"Completed: {report['completed_fixture_count']}/{report['fixture_count']}. "
        f"Failures: {report['failure_count']}.",
        "",
        "External payout: disabled.",
        "",
        "| Phase | Implementation | Configuration hash |",
        "| --- | --- | --- |",
    ]
    for phase in PHASES:
        policy = report["policy_manifest"][phase]
        lines.append(f"| {phase} | `{policy['implementation_id']}` | `{policy['configuration_hash']}` |")
    for section in report["partitions"]:
        lines += [
            "",
            f"## Partition `{section['partition']}`",
            "",
            f"Corpus: `{section['corpus_digest']}`",
            "",
            "| Fixture | Admission | Expected | State | Outcomes | Mismatches |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
        for item in section["fixtures"]:
            mismatches = ", ".join(item["mismatches"]) or "none"
            lines.append(
                f"| {item['label']} | {item['admission_decision']} | {item['expected_admission_decision']} "
                f"| {item['state']} | {item['phase_count']} | {mismatches} |"
            )
    lines += ["", "Production blockers: " + ", ".join(report["safe_blockers"]) + ".", ""]
    return "\n".join(lines)
