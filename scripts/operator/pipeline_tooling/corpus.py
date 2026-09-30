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
"""

from __future__ import annotations

import hashlib
import json
import re
import uuid
from pathlib import Path

from .environment import ROOT, run_child
from .errors import require

CORPUS_SCHEMA = "trace_commons.pipeline_corpus.v1"
PIN_SCHEMA = "trace_commons.pipeline_hf_corpus_pin.v1"

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


def _sha256(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


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
    actual = _sha256(data)
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
