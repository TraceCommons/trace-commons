#!/usr/bin/env python3
"""Tests for `near-benchmark-handoff.py` under pytest or standalone."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "near-benchmark-handoff.py"
FIXTURE = (
    HERE
    / "fixtures"
    / "near-benchmark-handoff"
    / "raw-envelope-export.json"
)


def _run(input_path: Path, output_dir: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--input",
            str(input_path),
            "--output-dir",
            str(output_dir),
        ],
        capture_output=True,
        text=True,
        check=False,
    )


def run_converter(input_path: Path, output_dir: Path) -> tuple[Path, dict[str, object]]:
    result = _run(input_path, output_dir)
    assert result.returncode == 0, result.stderr
    corpus = output_dir / "corpus.jsonl"
    manifest_path = output_dir / "handoff-manifest.json"
    assert corpus.is_file()
    assert manifest_path.is_file()
    return corpus, json.loads(manifest_path.read_text(encoding="utf-8"))


def test_writes_one_jsonl_line_per_item(tmp_path: Path) -> None:
    corpus, manifest = run_converter(FIXTURE, tmp_path)
    lines = corpus.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 2
    assert len(lines) == manifest["item_count"]


def test_every_jsonl_line_is_a_complete_envelope(tmp_path: Path) -> None:
    corpus, _ = run_converter(FIXTURE, tmp_path)
    for line in corpus.read_text(encoding="utf-8").splitlines():
        envelope = json.loads(line)
        assert "events" in envelope
        assert "submission_id" in envelope


def test_manifest_carries_provenance_unmodified(tmp_path: Path) -> None:
    _, manifest = run_converter(FIXTURE, tmp_path)
    source = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert manifest["export_id"] == source["export_id"]
    assert (
        manifest["source_submission_ids_hash"]
        == source["manifest"]["source_submission_ids_hash"]
    )


def test_manifest_sha256_matches_corpus_bytes(tmp_path: Path) -> None:
    corpus, manifest = run_converter(FIXTURE, tmp_path)
    digest = hashlib.sha256(corpus.read_bytes()).hexdigest()
    assert manifest["corpus_sha256"] == digest


def test_large_envelope_survives_round_trip(tmp_path: Path) -> None:
    corpus, _ = run_converter(FIXTURE, tmp_path)
    big = [
        json.loads(line)
        for line in corpus.read_text(encoding="utf-8").splitlines()
    ][1]
    assert len(big["events"][0]["redacted_content"]) >= 200_000


def test_refuses_non_low_privacy_risk_item(tmp_path: Path) -> None:
    source = json.loads(FIXTURE.read_text(encoding="utf-8"))
    source["items"][0]["privacy_risk"] = "medium"
    unsafe_input = tmp_path / "unsafe-export.json"
    unsafe_input.write_text(json.dumps(source), encoding="utf-8")
    output_dir = tmp_path / "output"

    result = _run(unsafe_input, output_dir)
    assert result.returncode != 0
    assert source["items"][0]["submission_id"] in result.stderr
    assert not (output_dir / "corpus.jsonl").exists()
    assert not (output_dir / "handoff-manifest.json").exists()


def main() -> int:
    failures = 0
    tests = [
        value
        for name, value in globals().items()
        if name.startswith("test_") and callable(value)
    ]
    for test in tests:
        try:
            with tempfile.TemporaryDirectory() as directory:
                test(Path(directory))
            print(f"ok   {test.__name__}")
        except AssertionError as error:
            failures += 1
            print(f"FAIL {test.__name__}: {error}")
        except Exception as error:  # noqa: BLE001
            failures += 1
            print(f"ERROR {test.__name__}: {type(error).__name__}: {error}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
