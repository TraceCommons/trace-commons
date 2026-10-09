"""The export, the report check, and the Markdown view of `pipeline.py compare`.

`compare` sends the traces of one pin through the old gate path of `main`
(side `baseline`) and through the versioned pipeline (side `candidate`). The
ignored Rust test `pipeline_compare_pg_tests::pipeline_compare_run` does the
run and writes one `trace_commons.pipeline_comparison_report.v1` report; this
module makes its input and checks its output.

`export_compare_corpus` drives the export binary
(`trace-commons-pipeline-corpus-export`) as `corpus.export_hf_corpus` does,
with `--with-events`: the output is one JSON line for each session
(`bootstrap-compare.jsonl`, `holdout-compare.jsonl`) and `source-manifest.json`.
It has its own output directory and step label for each `name`, so one run
can hold two exports, and it compares all five digests of the pin (the
declared privacy risks change only the corpus files; `--with-events`
changes the configuration, so `configuration_digest` is compared here too).

`validate_comparison_report` is to the comparison report what
`corpus.validate_report` is to the corpus report: only labels, hashes,
counts, and booleans (`corpus.safe_report_value`), the exact field set, the
digest over the canonical JSON, and counts that agree with each other.
"""

from __future__ import annotations

import hashlib
import json
import re

from .corpus import HF_CACHE_DIR, LOCAL_BLOCKERS, load_pin, safe_report_value
from .environment import run_child
from .errors import ToolingError, require
from .files import sha256_digest
from .results import canonical

REPORT_SCHEMA = "trace_commons.pipeline_comparison_report.v1"
# The check ids the harness may emit (`pipeline_compare_pg_tests`'s
# `COMPARE_CHECK_IDS`): a pin with a local directory, and a network pin.
CHECK_IDS = frozenset({"pipeline_comparison_local", "pipeline_comparison_hf"})
# `COMPARISON_BLOCKERS` in `versioned_pipeline_comparison.rs`. The last one
# is PC-D18: the harness removes the baseline tenant's derived files after
# each trace.
BLOCKERS = (*LOCAL_BLOCKERS, "deterministic_privacy_only", "baseline_derived_scan_removed")
# The report lists at most this many unexplained traces; `unexplained_total`
# counts each one.
UNEXPLAINED_LIST_LIMIT = 1000
# The failure labels of a comparison, as the harness prints them.
UNEXPLAINED_LABEL = "comparison_has_unexplained_differences"
BRANCH_LABEL = "comparison_gate_branch_not_exercised"
REFUSED_LABEL = "comparison_receipt_refused"  # PC-D19
ALIGNMENT_LABEL = "comparison_alignment_lost"  # PC-D20

# The digests of an export, in a pin and in `source-manifest.json`. A pin
# can lack the last three before its first export.
PIN_DIGEST_FIELDS = (
    "source_digest",
    "order_digest",
    "configuration_digest",
    "bootstrap_corpus_digest",
    "holdout_corpus_digest",
)

_HASH = re.compile(r"sha256:[a-f0-9]{64}\Z")

SIDES = ("baseline", "candidate")
_SIDE_COUNTS = (
    "admit",
    "quarantine",
    "reject",
    "refused",
    "other",
    "scored",
    "quality_passed",
    "quality_failed",
    "novelty_passed",
    "novelty_failed",
    "member",
    "not_member",
    "chunks_capped",
)
_FLOORS = ("perplexity_floor_micros", "tail_fraction_floor_micros", "novelty_floor_micros")
# The names that `unexplained_counts` and an `unexplained` entry can hold:
# the compared fields of `versioned_pipeline_comparison.rs`, and
# `record_pair` for two records that are not of one trace.
_COMPARED_FIELDS = frozenset(
    {
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
        "record_pair",
    }
)
# The gate branches that `ComparisonSummary::branch_gaps` can name.
_BRANCH_GAPS = frozenset(
    {
        "quality_passed_true",
        "quality_passed_false",
        "novelty_passed_true",
        "novelty_passed_false",
        "member_true",
        "member_false",
    }
)
# The rules that permit a difference (`PermittedDifference::ALL`, spec
# section 8.3). A report holds exactly this list: a rule that the harness
# does not write is not evidence.
PERMITTED_RULES = (
    {"rule": "medium_risk_privacy_review", "source": "ruling.PC-D22", "fields": ["admission"]},
)
_REPORT_HASHES = ("bundle_id", "package_hash", "configuration_digest", "dependency_digest", "records_digest")
_REPORT_COUNTS = ("trace_count", "compared_count", "equal_count", "unexplained_total")
_SKEWS = (None, "baseline_quality_floor")
# Exactly the fields `comparison_report` writes.
_REPORT_KEYS = frozenset(
    {
        "schema",
        "check_id",
        "scope",
        "production_ready",
        "external_payout_enabled",
        "partial",
        "skew",
        "safe_blockers",
        "pin",
        *_REPORT_HASHES,
        "floors",
        *_REPORT_COUNTS,
        "permitted_counts",
        "unexplained_counts",
        "unexplained",
        "first_unexplained_position",
        "alignment_lost_position",
        "excluded_rules",
        "permitted_rules",
        "distribution",
        "branch_gaps",
        "report_digest",
    }
)
_MARKDOWN_UNEXPLAINED_LIMIT = 20


def export_compare_corpus(run, pin_path, env, *, name, local_dir=None, release=False):
    """Runs the export with `--with-events` into `run.run_dir / "compare" /
    name`, with the step label `compare_export_<name>`, then checks the
    manifest against the pin. With `release=True`, the cargo command has
    `--release` (PC-D21). Returns `(bootstrap-compare.jsonl,
    holdout-compare.jsonl, source-manifest.json)`.

    The command is the command of `corpus.export_hf_corpus` plus
    `--with-events`, one `--session-name` for each name in the pin's
    `session_names`, and one `--declared-privacy-risk NAME=RISK` for each
    entry of its `declared_privacy_risk`. The two fields select files of a
    local directory, so a pin without `local_jsonl_dir` that has one is
    `comparison_pin_field_needs_local_dir`.

    Each of the five digests that the pin has must equal the manifest's
    (`hf_<field>_mismatch`), the manifest must say that the export had
    `--with-events`, and it must hold no raw trace text. An export that
    leaves no manifest, or one that is not a JSON object, is
    `comparison_manifest_malformed`."""
    pin = load_pin(pin_path)
    session_names = pin.get("session_names") or []
    declared_risks = pin.get("declared_privacy_risk") or {}
    require(
        bool(pin.get("local_jsonl_dir")) or not (session_names or declared_risks),
        "comparison_pin_field_needs_local_dir",
    )
    output_dir = run.run_dir / "compare" / name
    command = ["cargo", "run", "-q"]
    if release:
        command.append("--release")
    command += [
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
        "--with-events",
    ]
    for session_name in session_names:
        command += ["--session-name", str(session_name)]
    for session_name, risk in declared_risks.items():
        command += ["--declared-privacy-risk", f"{session_name}={risk}"]
    HF_CACHE_DIR.mkdir(parents=True, exist_ok=True)
    command += ["--cache-dir", str(HF_CACHE_DIR)]
    if local_dir is not None:
        command += ["--local-jsonl-dir", str(local_dir)]

    run_child(run, f"compare_export_{name}", command, env)

    manifest_path = output_dir / "source-manifest.json"
    try:
        manifest = json.loads(manifest_path.read_bytes())
    except (OSError, ValueError) as error:
        raise ToolingError("comparison_manifest_malformed") from error
    require(isinstance(manifest, dict), "comparison_manifest_malformed")
    for field in PIN_DIGEST_FIELDS:
        expected = pin.get(field)
        if expected is not None:
            require(manifest.get(field) == expected, f"hf_{field}_mismatch")
    source = manifest.get("source")
    require(isinstance(source, dict) and source.get("with_events") is True, "comparison_manifest_without_events")
    require(manifest.get("contains_raw_trace_text") is False, "hf_manifest_contains_raw_trace_text")

    return output_dir / "bootstrap-compare.jsonl", output_dir / "holdout-compare.jsonl", manifest_path


def file_digest(path):
    """`sha256:<hex>` of the file at `path`, read in blocks: the records
    file of a full run has two lines for each trace."""
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def validate_comparison_report(report):
    """A report of a run that found differences is valid; one that holds
    anything but labels, hashes, counts, and booleans, whose digest does not
    verify, or whose counts do not agree is not. A report that lacks a
    field, has a field that the harness does not write, or holds a value of
    the wrong type fails with `comparison_report_malformed`, never a
    traceback. So does a value that the harness can never write: a name
    outside the compared fields or the gap labels, an alignment position
    outside the compared pairs, or counts that do not agree on whether a
    trace is unexplained."""
    try:
        _validate_comparison_report(report)
    except (KeyError, TypeError, AttributeError, IndexError) as error:
        raise ToolingError("comparison_report_malformed") from error


def _is_count(value):
    return type(value) is int and value >= 0


def _is_hash(value):
    return isinstance(value, str) and _HASH.fullmatch(value) is not None


def _is_count_map(value):
    return isinstance(value, dict) and all(_is_count(count) for count in value.values())


def _is_text_list(value):
    return isinstance(value, list) and all(isinstance(item, str) for item in value)


def _is_name_list(value, names):
    """A list whose entries are all in the closed set `names`."""
    return _is_text_list(value) and set(value) <= names


def _validate_comparison_report(report):
    malformed = "comparison_report_malformed"
    require(isinstance(report, dict) and report.get("schema") == REPORT_SCHEMA, "unsupported_report_schema")
    require(report["scope"] == "local_test" and report["production_ready"] is False, "invalid_report_scope")
    require(report["external_payout_enabled"] is False, "payout_enabled")
    require(set(BLOCKERS).issubset(report["safe_blockers"]), "missing_local_blockers")
    require(safe_report_value(report) == report, "unsafe_report")
    require(set(report) == _REPORT_KEYS, malformed)
    require(report["check_id"] in CHECK_IDS, "invalid_report_check_id")
    pin = report["pin"]
    require(isinstance(pin, dict) and set(pin) == set(PIN_DIGEST_FIELDS), malformed)
    hashes = [report[field] for field in (*_REPORT_HASHES, "report_digest")] + list(pin.values())
    require(all(_is_hash(value) for value in hashes), "invalid_report_hash")
    unsigned = {key: value for key, value in report.items() if key != "report_digest"}
    require(sha256_digest(canonical(unsigned)) == report["report_digest"], "report_digest_mismatch")

    require(isinstance(report["partial"], bool) and report["skew"] in _SKEWS, malformed)
    require(_is_text_list(report["safe_blockers"]) and _is_name_list(report["branch_gaps"], _BRANCH_GAPS), malformed)
    floors = report["floors"]
    require(
        isinstance(floors, dict) and set(floors) == set(_FLOORS) and all(_is_count(floors[key]) for key in _FLOORS),
        malformed,
    )
    require(all(_is_count(report[field]) for field in _REPORT_COUNTS), malformed)
    require(_is_count_map(report["permitted_counts"]) and _is_count_map(report["unexplained_counts"]), malformed)
    require(set(report["unexplained_counts"]) <= _COMPARED_FIELDS, malformed)
    require(set(report["permitted_counts"]) <= {rule["rule"] for rule in PERMITTED_RULES}, malformed)
    require(report["permitted_rules"] == list(PERMITTED_RULES), malformed)
    for field in ("first_unexplained_position", "alignment_lost_position"):
        require(report[field] is None or _is_count(report[field]), malformed)
    # The pair that lost the alignment is the last compared pair.
    lost = report["alignment_lost_position"]
    require(lost is None or lost < report["compared_count"], malformed)
    # The total, the field counts, and the first position agree on whether
    # a trace is unexplained.
    none_unexplained = report["unexplained_total"] == 0
    require((not report["unexplained_counts"]) is none_unexplained, malformed)
    require((report["first_unexplained_position"] is None) is none_unexplained, malformed)
    distribution = report["distribution"]
    require(isinstance(distribution, dict) and set(distribution) == set(SIDES), malformed)
    for side in SIDES:
        counts = distribution[side]
        require(
            isinstance(counts, dict)
            and set(counts) == set(_SIDE_COUNTS)
            and all(_is_count(counts[key]) for key in _SIDE_COUNTS),
            malformed,
        )
    require(isinstance(report["excluded_rules"], list), malformed)
    for rule in report["excluded_rules"]:
        require(
            isinstance(rule, dict)
            and set(rule) == {"rule", "source", "fields"}
            and isinstance(rule["rule"], str)
            and isinstance(rule["source"], str)
            and _is_text_list(rule["fields"]),
            malformed,
        )
    unexplained = report["unexplained"]
    require(isinstance(unexplained, list), malformed)
    for entry in unexplained:
        require(
            isinstance(entry, dict)
            and set(entry) == {"position", "trace_hash", "fields"}
            and _is_count(entry["position"])
            and _is_hash(entry["trace_hash"])
            and _is_name_list(entry["fields"], _COMPARED_FIELDS)
            and len(entry["fields"]) > 0,
            malformed,
        )

    compared = report["compared_count"]
    require(
        report["equal_count"] + sum(report["permitted_counts"].values()) + report["unexplained_total"] == compared,
        "comparison_count_mismatch",
    )
    require(
        len(unexplained) == min(report["unexplained_total"], UNEXPLAINED_LIST_LIMIT), "comparison_count_mismatch"
    )
    require(compared <= report["trace_count"], "comparison_count_mismatch")
    require(report["partial"] is (compared < report["trace_count"]), "comparison_partial_mismatch")


def markdown(report):
    """The Markdown view of a valid report: the check, the bundle, the pin,
    the floors, the counts, the distribution of the two sides, the fields
    that differ, and the first unexplained traces."""
    pin = report["pin"]
    floors = report["floors"]
    distribution = report["distribution"]
    lines = [
        "# Pipeline comparison report",
        "",
        f"Check: `{report['check_id']}`",
        "",
        f"Bundle: `{report['bundle_id']}`",
        "",
        f"Partial run: {'yes' if report['partial'] else 'no'}. Skew: {report['skew'] or 'none'}.",
        "",
        "## Pin",
        "",
        "| Digest | Value |",
        "| --- | --- |",
        *(f"| {field} | `{pin[field]}` |" for field in PIN_DIGEST_FIELDS),
        "",
        "## Floors",
        "",
        "| Floor | Micros |",
        "| --- | --- |",
        *(f"| {field} | {floors[field]} |" for field in _FLOORS),
        "",
        "## Counts",
        "",
        "| Count | Traces |",
        "| --- | --- |",
        f"| In the pin | {report['trace_count']} |",
        f"| Compared | {report['compared_count']} |",
        f"| Equal | {report['equal_count']} |",
        f"| Permitted | {sum(report['permitted_counts'].values())} |",
        f"| Unexplained | {report['unexplained_total']} |",
        "",
        "## Permitted differences",
        "",
        *(
            f"- `{rule['rule']}` ({rule['source']}): {report['permitted_counts'].get(rule['rule'], 0)}"
            for rule in report["permitted_rules"]
        ),
        "",
        "## Distribution",
        "",
        "| Count | Baseline | Candidate |",
        "| --- | --- | --- |",
        *(f"| {key} | {distribution['baseline'][key]} | {distribution['candidate'][key]} |" for key in _SIDE_COUNTS),
        "",
        "## Unexplained fields",
        "",
    ]
    counts = report["unexplained_counts"]
    lines += [f"- `{field}`: {counts[field]}" for field in sorted(counts)] or ["None."]
    shown = report["unexplained"][:_MARKDOWN_UNEXPLAINED_LIMIT]
    if shown:
        lines += [
            "",
            f"## Unexplained traces (the first {len(shown)} of {report['unexplained_total']})",
            "",
            "| Position | Trace hash | Fields |",
            "| --- | --- | --- |",
            *(f"| {entry['position']} | `{entry['trace_hash']}` | {', '.join(entry['fields'])} |" for entry in shown),
        ]
    if report["alignment_lost_position"] is not None:
        lines += [
            "",
            f"The run stopped at position {report['alignment_lost_position']}: "
            f"after this pair, the two indexes can differ (`{ALIGNMENT_LABEL}`).",
        ]
    if report["branch_gaps"]:
        lines += ["", "Gate branches with no evidence: " + ", ".join(report["branch_gaps"]) + "."]
    lines += ["", "Production blockers: " + ", ".join(report["safe_blockers"]) + ".", ""]
    return "\n".join(lines)
