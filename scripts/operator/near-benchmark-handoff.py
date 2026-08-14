#!/usr/bin/env python3
"""Package a raw-envelope export for the NEAR benchmark handoff.

Expected input is the JSON response from
`POST /v1/workers/raw-envelope-export`: top-level export and audit IDs, a
provenance manifest, and `items` whose `envelope` fields are complete
TraceContributionEnvelope documents. The converter writes one envelope per
line to `corpus.jsonl` and a hash-bound `handoff-manifest.json` beside it.

Only standard-library modules are used.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any


class ConversionError(Exception):
    """Operator-actionable input or packaging failure."""


def _require_mapping(value: object, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ConversionError(f"{field} must be a JSON object")
    return value


def _validated_items(source: dict[str, Any]) -> list[dict[str, Any]]:
    items = source.get("items")
    if not isinstance(items, list):
        raise ConversionError("items must be a JSON array")
    if source.get("item_count") != len(items):
        raise ConversionError(
            "item_count does not match the number of exported items"
        )

    validated: list[dict[str, Any]] = []
    for index, value in enumerate(items):
        item = _require_mapping(value, f"items[{index}]")
        submission_id = item.get("submission_id")
        if not isinstance(submission_id, str) or not submission_id:
            raise ConversionError(
                f"items[{index}].submission_id must be a non-empty string"
            )
        risk = item.get("privacy_risk")
        if risk != "low":
            raise ConversionError(
                f"submission {submission_id} has privacy_risk={risk}; "
                "only low-risk records may be handed off"
            )
        envelope = _require_mapping(item.get("envelope"), f"items[{index}].envelope")
        if envelope.get("submission_id") != submission_id:
            raise ConversionError(
                f"submission {submission_id} does not match its envelope submission_id"
            )
        if not isinstance(envelope.get("events"), list):
            raise ConversionError(
                f"submission {submission_id} envelope.events must be a JSON array"
            )
        validated.append(item)
    return validated


def convert(source: dict[str, Any], output_dir: Path) -> dict[str, Any]:
    """Write corpus and provenance artifacts, returning the handoff manifest."""

    manifest_source = _require_mapping(source.get("manifest"), "manifest")
    items = _validated_items(source)
    output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = output_dir / "corpus.jsonl"

    entries: list[dict[str, Any]] = []
    with corpus_path.open("w", encoding="utf-8", newline="\n") as handle:
        for item in items:
            handle.write(
                json.dumps(item["envelope"], separators=(",", ":")) + "\n"
            )
            entries.append(
                {
                    "submission_id": item["submission_id"],
                    "privacy_risk": item["privacy_risk"],
                    "redaction_counts": item.get("redaction_counts", {}),
                }
            )

    digest = hashlib.sha256(corpus_path.read_bytes()).hexdigest()
    manifest = {
        "export_id": source.get("export_id"),
        "audit_event_id": source.get("audit_event_id"),
        "source_submission_ids_hash": manifest_source.get(
            "source_submission_ids_hash"
        ),
        "item_count": len(entries),
        # Distinct scopes, not one entry per trace. The export manifest carries a
        # scope per item, which for a single-scope corpus repeats one value
        # hundreds of times and buries the fact being recorded.
        "consent_basis": sorted(set(manifest_source.get("consent_scopes", []))),
        "corpus_sha256": digest,
        "items": entries,
    }
    (output_dir / "handoff-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    try:
        source = _require_mapping(
            json.loads(args.input.read_text(encoding="utf-8")), "input"
        )
        manifest = convert(source, args.output_dir)
    except (ConversionError, OSError, json.JSONDecodeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(
        "# NearBenchmarkHandoffComplete "
        f"items={manifest['item_count']} "
        f"corpus_sha256={manifest['corpus_sha256']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
