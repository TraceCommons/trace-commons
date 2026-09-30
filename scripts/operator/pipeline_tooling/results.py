"""PipelineCheckResult loading and validation.

Mirrors `PipelineCheckResult` and its `validate()` in
`crates/trace-commons-server/src/versioned_pipeline_qualification.rs`: the
same schema name, the same exact key set (`deny_unknown_fields`), the same
label and hash shapes. `canonical()` matches
`trace_commons_protocol::canonical_json::to_canonical_vec` for the ASCII
keys this tooling uses (see the plan's global constraints).

`validate_evidence` ports `ef97a459:scripts/operator/lab/lab.py` lines
219-235, tightened to labels, `sha256:` hashes, and ISO observation times.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Optional

from .errors import ToolingError, require

SCHEMA = "trace_commons.pipeline_check_result.v1"

_LABEL = re.compile(r"[a-z0-9_]{1,64}\Z")
_HASH = re.compile(r"sha256:[0-9a-f]{64}\Z")
_STATUSES = ("pass", "fail", "blocked")

REQUIRED_KEYS = frozenset(
    {
        "schema",
        "run_id",
        "check_id",
        "status",
        "code_revision_hash",
        "package_hash",
        "configuration_digest",
        "dependency_digest",
        "observed_at",
        "evidence_hash",
        "safe_blockers",
    }
)

# Evidence values: a looser label (repository paths, key identifiers) than
# operational labels, matching lab.py's LABEL alphabet.
_EVIDENCE_LABEL = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")
_EVIDENCE_TIMESTAMP = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+(?:Z|[+-][0-9:]+)\Z")
PRIVATE_EVIDENCE_FIELDS = frozenset(
    {"input", "text", "trace_text", "secret", "secret_probe", "token", "account_id", "email"}
)
_SECRET_PREFIXES = ("ghp_", "github_pat_", "sk-")


def canonical(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
    ).encode()


def _digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def validate_evidence(value):
    """Refuses secret-shaped strings, the private field names, floats, and
    anything else outside labels/hashes/ISO timestamps/structured values of
    those. Raises `ToolingError` on the first violation found."""
    if isinstance(value, dict):
        require(not PRIVATE_EVIDENCE_FIELDS.intersection(value), "unsafe_evidence_field")
        for key, item in value.items():
            require(isinstance(key, str) and _EVIDENCE_LABEL.fullmatch(key) is not None, "unsafe_evidence_field")
            validate_evidence(item)
    elif isinstance(value, list):
        for item in value:
            validate_evidence(item)
    elif isinstance(value, str):
        require(not value.startswith(_SECRET_PREFIXES), "unsafe_evidence_value")
        require("@" not in value, "unsafe_evidence_value")
        require(len(value) <= 128, "unsafe_evidence_value")
        require(
            _EVIDENCE_TIMESTAMP.fullmatch(value) is not None or _EVIDENCE_LABEL.fullmatch(value) is not None,
            "unsafe_evidence_value",
        )
    elif isinstance(value, bool):
        pass
    elif isinstance(value, int):
        pass
    elif value is None:
        pass
    else:
        # Covers float (and any other unstructured type): the Rust schema
        # rejects non-integral numbers in evidence for the same reason
        # (evidence_hash's `check` in versioned_pipeline_qualification.rs).
        raise ToolingError("unsafe_evidence_value")


@dataclass(frozen=True)
class CheckResult:
    schema: str
    run_id: str
    check_id: str
    status: str
    code_revision_hash: str
    package_hash: Optional[str]
    configuration_digest: Optional[str]
    dependency_digest: Optional[str]
    observed_at: datetime
    evidence_hash: str
    safe_blockers: tuple


def _parse_observed_at(value):
    if not isinstance(value, str):
        raise ToolingError("check_result_schema_invalid")
    text = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError as error:
        raise ToolingError("check_result_schema_invalid") from error
    if parsed.tzinfo is None:
        raise ToolingError("check_result_schema_invalid")
    return parsed.astimezone(timezone.utc)


def _validate_schema(raw):
    require(isinstance(raw, dict) and set(raw) == REQUIRED_KEYS, "check_result_schema_invalid")
    require(raw.get("schema") == SCHEMA, "check_result_schema_invalid")
    require(isinstance(raw["run_id"], str) and _LABEL.fullmatch(raw["run_id"]) is not None, "check_result_schema_invalid")
    require(isinstance(raw["check_id"], str) and _LABEL.fullmatch(raw["check_id"]) is not None, "check_result_schema_invalid")
    require(raw.get("status") in _STATUSES, "check_result_schema_invalid")
    require(
        isinstance(raw["code_revision_hash"], str) and _HASH.fullmatch(raw["code_revision_hash"]) is not None,
        "check_result_schema_invalid",
    )
    require(
        isinstance(raw["evidence_hash"], str) and _HASH.fullmatch(raw["evidence_hash"]) is not None,
        "check_result_schema_invalid",
    )
    for key in ("package_hash", "configuration_digest", "dependency_digest"):
        value = raw[key]
        require(value is None or (isinstance(value, str) and _HASH.fullmatch(value) is not None), "check_result_schema_invalid")
    blockers = raw["safe_blockers"]
    require(
        isinstance(blockers, list)
        and all(isinstance(item, str) and _LABEL.fullmatch(item) is not None for item in blockers),
        "check_result_schema_invalid",
    )


def load_results(run):
    """Loads every `results/*.result.json` in `run.results_dir`, validating
    each against the Rust schema and its paired evidence file's hash.
    Structural problems (schema-invalid, tampered or missing evidence) raise
    here; relational problems (foreign run, stale, failed, ...) are
    `require_current_pass_results`'s job."""
    results = {}
    results_dir = run.results_dir
    if not results_dir.is_dir():
        return results
    for result_path in sorted(results_dir.glob("*.result.json")):
        raw = json.loads(result_path.read_text())
        _validate_schema(raw)
        check_id = raw["check_id"]

        evidence_path = results_dir / f"{check_id}.evidence.json"
        require(evidence_path.is_file(), "check_evidence_missing")
        evidence_value = json.loads(evidence_path.read_text())
        require(_digest(canonical(evidence_value)) == raw["evidence_hash"], "check_evidence_hash_mismatch")

        results[check_id] = CheckResult(
            schema=raw["schema"],
            run_id=raw["run_id"],
            check_id=check_id,
            status=raw["status"],
            code_revision_hash=raw["code_revision_hash"],
            package_hash=raw["package_hash"],
            configuration_digest=raw["configuration_digest"],
            dependency_digest=raw["dependency_digest"],
            observed_at=_parse_observed_at(raw["observed_at"]),
            evidence_hash=raw["evidence_hash"],
            safe_blockers=tuple(raw["safe_blockers"]),
        )
    return results


def require_current_pass_results(run, results, required):
    """`required` maps check id to `checks.CheckSpec`. Every required check
    must have a passing, current result from this exact run, against this
    exact code revision, with its required digests present."""
    from datetime import datetime as _datetime

    now = _datetime.now(timezone.utc)
    for check_id, spec in required.items():
        result = results.get(check_id)
        if result is None:
            raise ToolingError(f"check_result_missing:{check_id}")
        if result.run_id != run.run_id:
            raise ToolingError("check_result_foreign_run")
        if result.code_revision_hash != run.code_revision_hash:
            raise ToolingError("check_result_foreign_revision")
        if result.observed_at < run.started_at or result.observed_at > now:
            raise ToolingError("check_result_stale")
        if result.status == "blocked":
            raise ToolingError(f"check_result_blocked:{check_id}")
        if result.status == "fail":
            raise ToolingError(f"check_result_failed:{check_id}")
        if spec.digests_required and (
            result.package_hash is None
            or result.configuration_digest is None
            or result.dependency_digest is None
        ):
            raise ToolingError(f"check_result_digest_missing:{check_id}")
