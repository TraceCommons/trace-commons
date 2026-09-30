"""The local pipeline lab catalog (P4-D19).

Ports `archive` and `update_catalog` from `ef97a459:scripts/operator/lab/lab.py`
lines 254-323: `.local/pipeline-lab-catalog.json`
(`trace_commons.pipeline_lab_catalog.v1`) lists each bundle's archived
reports, and every archived file is a content-addressed record under
`lab-records/` that is written once and never changed. Only `run --archive`
(and, from Task 11, `qualify --archive`) calls `update_catalog`; a routine run
writes only its run directory and the latest bounded report under `.local/`.

The primary report here is a `trace_commons.pipeline_corpus_report.v1`
(validated with `corpus.validate_report`). The signed package and trusted key
of a package run are not copied into the catalog: the report already names
the package by its hash.
"""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import tempfile
from pathlib import Path

from .corpus import validate_report
from .errors import require
from .results import canonical, validate_evidence

CATALOG_SCHEMA = "trace_commons.pipeline_lab_catalog.v1"
CATALOG_NAME = "pipeline-lab-catalog.json"

# Evidence that may be archived beside a report (P4-D19, Task 11). Records
# are evidence, never corpus inputs.
RECORD_SCHEMAS = frozenset(
    {
        "trace_commons.pipeline_qualification_report.v1",
        "trace_commons.pipeline_corpus_report.v1",
        "trace_commons.pipeline_hf_corpus_manifest.v1",
    }
)


def _digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _atomic_write(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def archive(catalog_path, data, kind):
    """Writes `data` once as `lab-records/<kind>-<sha256>.json` beside the
    catalog and returns that path relative to the catalog's directory. The
    same digest with different bytes is refused."""
    identity = _digest(data)
    destination = Path(catalog_path).parent / "lab-records" / f"{kind}-{identity[7:]}.json"
    if destination.exists():
        require(destination.read_bytes() == data, "immutable_record_conflict")
    else:
        _atomic_write(destination, data)
    return destination.relative_to(Path(catalog_path).parent).as_posix()


def update_catalog(catalog_path, report_path, records=()):
    """Validates the report and every record before changing any file, then
    archives them and adds the report to its bundle's catalog entry, under
    an exclusive lock. Archiving the same report twice leaves the catalog
    unchanged."""
    catalog_path = Path(catalog_path)
    report = json.loads(Path(report_path).read_bytes())
    validate_report(report)
    extra = []
    for record in records:
        value = json.loads(Path(record).read_bytes())
        require(isinstance(value, dict) and value.get("schema") in RECORD_SCHEMAS, "unsupported_development_record")
        validate_evidence(value)
        extra.append(canonical(value) + b"\n")

    catalog_path.parent.mkdir(parents=True, exist_ok=True)
    with catalog_path.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if catalog_path.exists():
            catalog = json.loads(catalog_path.read_bytes())
        else:
            catalog = {"schema": CATALOG_SCHEMA, "bundles": []}
        require(catalog.get("schema") == CATALOG_SCHEMA, "unsupported_catalog_schema")
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
        _atomic_write(catalog_path, json.dumps(catalog, indent=2, sort_keys=True, allow_nan=False).encode() + b"\n")
    return catalog
