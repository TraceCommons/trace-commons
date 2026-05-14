#!/usr/bin/env python3
"""Smoke test for `bakeoff-progress.py`.

Runs the script against the bundled fixture and compares its text output to
the expected snapshot. Designed to work either under pytest:

    pytest scripts/operator/test_bakeoff_progress.py

or as a standalone script:

    python3 scripts/operator/test_bakeoff_progress.py

Exits non-zero on any failed assertion.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "bakeoff-progress.py"
FIXTURE = HERE / "fixtures" / "bakeoff-progress-sample.log"

# Fixed wall-clock "now" so the script's elapsed-time / ETA output is
# deterministic. Chosen to fall after the last load_done of the 3rd
# candidate, giving us a mid-scoring SCORING row plus a PENDING slot.
NOW = "2026-05-14T14:35:00Z"

EXPECTED_TEXT = (
    "A2.6 BAKE-OFF SUMMARY\n"
    "Started: 2026-05-14 11:11:37 UTC\n"
    "Elapsed: 3h 23m\n"
    "Candidates: 2 of 4 complete\n"
    "\n"
    "llama-3.1-8b-instruct:  DONE  AUC=0.342  para_delta=0.829  "
    "throughput=292tps  vram=19.1GB  (scored in 83.1m)\n"
    "qwen3-8b-base:          DONE  AUC=0.355  para_delta=0.811  "
    "throughput=305tps  vram=18.5GB  (scored in 75.5m)\n"
    "qwen3.6-27b-dense:      SCORING  elapsed 32.2m\n"
    "(pending slot 4):       PENDING\n"
    "\n"
    "ETA: ~16:46 UTC (extrapolated from 2 completed candidates)\n"
)


def _run(args: list) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
    )


def test_text_output_matches_snapshot() -> None:
    result = _run(["--log", str(FIXTURE), "--now", NOW])
    assert result.returncode == 0, (
        f"non-zero exit: {result.returncode}\nstderr:\n{result.stderr}"
    )
    assert result.stdout == EXPECTED_TEXT, (
        "text output did not match snapshot.\n"
        f"--- expected ---\n{EXPECTED_TEXT}\n"
        f"--- actual ---\n{result.stdout}"
    )


def test_json_output_is_well_formed() -> None:
    result = _run(["--log", str(FIXTURE), "--now", NOW, "--out", "json"])
    assert result.returncode == 0, result.stderr
    payload = json.loads(result.stdout)
    assert payload["candidate_count"] == 4
    assert payload["completed"] == 2
    assert payload["bakeoff_done"] is False
    ids = [c["id"] for c in payload["candidates"]]
    assert ids == [
        "llama-3.1-8b-instruct",
        "qwen3-8b-base",
        "qwen3.6-27b-dense",
    ], ids
    statuses = {c["id"]: c["status"] for c in payload["candidates"]}
    assert statuses["llama-3.1-8b-instruct"] == "done"
    assert statuses["qwen3-8b-base"] == "done"
    assert statuses["qwen3.6-27b-dense"] == "scoring"


def test_missing_log_returns_nonzero() -> None:
    result = _run(["--log", "/definitely/does/not/exist.log"])
    assert result.returncode != 0
    assert "log not found" in result.stderr


def main() -> int:
    failures = 0
    for name, fn in list(globals().items()):
        if not name.startswith("test_") or not callable(fn):
            continue
        try:
            fn()
            print(f"ok   {name}")
        except AssertionError as exc:
            failures += 1
            print(f"FAIL {name}: {exc}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
