"""The one SHA-256 digest helper and the one atomic file writer the tooling
shares (`pipeline.py`, `results`, `corpus`, `catalog`)."""

from __future__ import annotations

import hashlib
import os
import tempfile
from pathlib import Path


def sha256_digest(data):
    """`sha256:<hex>` of `data` (bytes)."""
    return "sha256:" + hashlib.sha256(data).hexdigest()


def atomic_write(path, data):
    """Writes `data` to a fresh temporary file beside `path`, flushes and
    fsyncs it, then renames it onto `path`. A reader never sees a partial
    file, and a crash leaves either the old file or the new one."""
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
