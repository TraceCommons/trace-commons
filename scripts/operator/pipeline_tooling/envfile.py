"""The deployment's env file, read for the production checks (spec
2026-10-08, B-D1 and B-D5).

`pipeline.py package --bundle production` builds the production package
from the same variables the deployed ingest reads, and `promote
package-checks` gives the harness the NEAR AI endpoint, key and embedder
cache the real dependencies need. Neither passes the file through: each
takes only its own allowlisted variables (`PACKAGE_VARIABLES`,
`PACKAGE_CHECK_VARIABLES`), so a database URL, a bucket, a master key, or
anything else the file holds never reaches a child. No value, line or path
of the file reaches a label or the terminal.

The format is systemd's `EnvironmentFile=`, as the pilot uses it: `KEY=VALUE`
lines, blank lines and `#` comments, and a value optionally wrapped in one
pair of matching quotes.
"""

from __future__ import annotations

import re
from pathlib import Path

from .errors import ToolingError, require

_KEY = re.compile(r"[A-Z_][A-Z0-9_]*\Z")

# What the production package is built from: the scorer and embedder
# descriptors (spec A-D4, A-D5) and `main`'s gate configuration, exactly the
# variables `pipeline_main_gate_config_from_env` and the descriptor parsers
# of `production_assembly.rs` read.
PACKAGE_VARIABLES = (
    "TRACE_COMMONS_NEAR_AI_MODEL",
    "TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF",
    "TRACE_COMMONS_EMBEDDER_MODEL_ID",
    "TRACE_COMMONS_VECTOR_INDEX_DIM",
    "TRACE_COMMONS_EMBEDDER_MAX_TOKENS",
    "TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM",
    "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS",
    "TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS",
    "TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS",
    "TRACE_COMMONS_GATE_TOP_K",
    "TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS",
    "TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS",
    "TRACE_COMMONS_GATE_CHUNK_CAP",
    "TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS",
    "TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS",
    "TRACE_COMMONS_GATE_QUALIFYING_CHUNK_FLOOR_MICROS",
    "TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA",
)

# What a production-mode harness needs besides the package, which already
# binds the model, the cutoff and the embedder (`harness_dependencies_from_env`
# in `versioned_pipeline_harness.rs`).
PACKAGE_CHECK_VARIABLES = (
    "TRACE_COMMONS_NEAR_AI_BASE_URL",
    "TRACE_COMMONS_NEAR_AI_API_KEY",
    "TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS",
    "TRACE_COMMONS_EMBEDDER_CACHE_DIR",
)


def read_env_file(path):
    """Every `KEY=VALUE` of the file at `path`. An unreadable file is
    `env_file_unreadable`; a line that is not a comment, blank, or a valid
    assignment is `env_file_invalid`."""
    try:
        text = Path(path).read_text()
    except (OSError, UnicodeDecodeError) as error:
        raise ToolingError("env_file_unreadable") from error
    values = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        key = key.strip()
        require(separator == "=" and _KEY.fullmatch(key) is not None, "env_file_invalid")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
            value = value[1:-1]
        values[key] = value
    return values


def allowlisted(values, names):
    """The variables of `values` named in `names`, and no others."""
    return {name: values[name] for name in names if name in values}
