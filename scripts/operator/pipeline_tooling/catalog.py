"""The local pipeline lab catalog (P4-D19).

Ports `archive` and `update_catalog` from `ef97a459:scripts/operator/lab/lab.py`
lines 254-323: `.local/pipeline-lab-catalog.json`
(`trace_commons.pipeline_lab_catalog.v1`) lists what was archived, and every
archived file is a content-addressed record under `lab-records/` that is
written once and never changed. Only `run --archive` and `qualify --archive`
call `update_catalog`; a routine run writes only its run directory and the
latest bounded report under `.local/`.

The primary report decides the entry:

- a `trace_commons.pipeline_corpus_report.v1` (`run --archive`, validated
  with `corpus.validate_report`) is added to its bundle's entry under
  `bundles`, as in the port;
- a `trace_commons.pipeline_qualification_report.v1` (`qualify --archive`,
  ruling T11-1, validated with `report.validate_qualification_report`) is
  added under `qualifications`, with its records: the corpus reports of the
  run and the HF corpus manifest. Each of those must belong to the report
  (the port's `qualification_*_mismatch` checks, inverted).

Records are evidence, never corpus inputs, and each is validated for its own
schema before any file changes. The signed package and trusted key of a
package run are not copied into the catalog: the report already names the
package by its hash.
"""

from __future__ import annotations

import fcntl
import json
from pathlib import Path

from .corpus import MANIFEST_SCHEMA, REPORT_SCHEMA as CORPUS_REPORT_SCHEMA, validate_hf_manifest, validate_report
from .errors import ToolingError, require
from .files import atomic_write, sha256_digest
from .report import REPORT_SCHEMA as QUALIFICATION_REPORT_SCHEMA, validate_qualification_report
from .results import canonical, validate_evidence

CATALOG_SCHEMA = "trace_commons.pipeline_lab_catalog.v1"
CATALOG_NAME = "pipeline-lab-catalog.json"

# Evidence that may be archived beside a primary report (P4-D19, Task 11).
RECORD_SCHEMAS = frozenset({QUALIFICATION_REPORT_SCHEMA, CORPUS_REPORT_SCHEMA, MANIFEST_SCHEMA})


def archive(catalog_path, data, kind):
    """Writes `data` once as `lab-records/<kind>-<sha256>.json` beside the
    catalog and returns that path relative to the catalog's directory. The
    same digest with different bytes is refused."""
    identity = sha256_digest(data)
    destination = Path(catalog_path).parent / "lab-records" / f"{kind}-{identity[7:]}.json"
    if destination.exists():
        require(destination.read_bytes() == data, "immutable_record_conflict")
    else:
        atomic_write(destination, data)
    return destination.relative_to(Path(catalog_path).parent).as_posix()


def _validate_record(value):
    """Each record schema's own check. A corpus report and a qualification
    report pass the evidence validator inside their own validators; the HF
    manifest has an exact shape instead, because its dataset name holds a
    `/` that evidence labels do not allow."""
    require(isinstance(value, dict) and value.get("schema") in RECORD_SCHEMAS, "unsupported_development_record")
    if value["schema"] == CORPUS_REPORT_SCHEMA:
        validate_report(value)
        validate_evidence(value)
    elif value["schema"] == QUALIFICATION_REPORT_SCHEMA:
        validate_qualification_report(value)
    else:
        validate_hf_manifest(value)


def _require_records_belong(report, records):
    """Every corpus report named in a qualification report's inputs by its
    check id, bundle, package, and report digest; the HF manifest by the two
    corpus digests of the report's HF run."""
    runs = report["inputs"]["corpus_runs"]
    for value in records:
        if value["schema"] == CORPUS_REPORT_SCHEMA:
            require(
                any(
                    (run["check_id"], run["bundle_id"], run["package_hash"], run["report_digest"])
                    == (value["check_id"], value["bundle_id"], value["package_hash"], value["report_digest"])
                    for run in runs
                ),
                "qualification_corpus_mismatch",
            )
        elif value["schema"] == MANIFEST_SCHEMA:
            digests = [value["bootstrap_corpus_digest"], value["holdout_corpus_digest"]]
            require(
                any(run["check_id"] == "pipeline_http_corpus_hf_local" and run["corpus_digests"] == digests
                    for run in runs),
                "qualification_manifest_mismatch",
            )


def _read_json(path):
    try:
        return json.loads(Path(path).read_bytes())
    except (OSError, ValueError) as error:
        raise ToolingError("catalog_input_unreadable") from error


def update_catalog(catalog_path, report_path, records=()):
    """Validates the primary report and every record before changing any
    file, then archives them and adds the report to the catalog, under an
    exclusive lock. Archiving the same report and records twice leaves the
    catalog unchanged."""
    catalog_path = Path(catalog_path)
    report = _read_json(report_path)
    qualification = isinstance(report, dict) and report.get("schema") == QUALIFICATION_REPORT_SCHEMA
    if qualification:
        validate_qualification_report(report)
    else:
        validate_report(report)
    values = [_read_json(record) for record in records]
    for value in values:
        _validate_record(value)
    if qualification:
        _require_records_belong(report, values)
    extra = [canonical(value) + b"\n" for value in values]

    catalog_path.parent.mkdir(parents=True, exist_ok=True)
    with catalog_path.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if catalog_path.exists():
            catalog = json.loads(catalog_path.read_bytes())
        else:
            catalog = {"schema": CATALOG_SCHEMA, "bundles": []}
        require(catalog.get("schema") == CATALOG_SCHEMA, "unsupported_catalog_schema")
        if qualification:
            _add_qualification(catalog_path, catalog, report, extra)
        else:
            _add_corpus_report(catalog_path, catalog, report, extra)
        atomic_write(catalog_path, json.dumps(catalog, indent=2, sort_keys=True, allow_nan=False).encode() + b"\n")
    return catalog


def _add_corpus_report(catalog_path, catalog, report, extra):
    bundles = catalog["bundles"]
    entry = next((item for item in bundles if item["bundle_id"] == report["bundle_id"]), None)
    if entry is None:
        entry = {"bundle_id": report["bundle_id"], "development_records": []}
        bundles.append(entry)
    entry.update(
        configuration_digest=report["configuration_digest"],
        production_ready=False,
        safe_blockers=report["safe_blockers"],
    )
    record = {
        "check_id": report["check_id"],
        "report_digest": report["report_digest"],
        "corpus_digests": [section["corpus_digest"] for section in report["partitions"]],
        "configuration_digest": report["configuration_digest"],
        "package_hash": report["package_hash"],
        "status": "pass" if report["failure_count"] == 0 else "fail",
    }
    record["report"] = archive(catalog_path, canonical(report) + b"\n", "report")
    paths = [record["report"]]
    for data in extra:
        paths.append(archive(catalog_path, data, "evidence"))
    entry["development_records"] = sorted(set(entry["development_records"] + paths))
    previous = entry.setdefault("reports", [])
    if record not in previous:
        previous.append(record)
    previous.sort(key=lambda item: item["report_digest"])
    bundles.sort(key=lambda item: item["bundle_id"])


def _add_qualification(catalog_path, catalog, report, extra):
    record = {
        "evidence_hash": report["evidence_hash"],
        "code_revision_hash": report["inputs"]["code_revision_hash"],
        "status": report["status"],
        "production_promotion_ready": False,
        "safe_blockers": report["safe_blockers"],
        "report": archive(catalog_path, canonical(report) + b"\n", "qualification"),
    }
    record["records"] = sorted({archive(catalog_path, data, "evidence") for data in extra})
    previous = catalog.setdefault("qualifications", [])
    if record not in previous:
        previous.append(record)
    previous.sort(key=lambda item: (item["evidence_hash"], item["report"]))
