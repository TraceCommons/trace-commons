#!/usr/bin/env python3
"""Write the release manifest that `ironwire update` and install.sh both read.

One source of truth for "what is latest". `docs/UPDATES.md` fixes the schema and
the rule that matters: IronWire **notifies only**. This document tells a client
that a newer version exists and which command belongs to its install method. It
does not, and structurally cannot, tell a client where to download anything —
there is no URL field, by design, so a compromised manifest cannot redirect a
user's install (`docs/TRUST.md` I2 applies the same reasoning to quirks).

Run locally:
    python packaging/write_manifest.py --version 0.1.0 --dist dist --out manifest.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

# Below this, a build is old enough that providers may have changed in ways it
# does not handle, and `ironwire status` says so rather than staying quiet.
# Raised deliberately, per release, never automatically.
MINIMUM_SUPPORTED = "0.1.0"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--summary", default="")
    args = parser.parse_args()

    # Checksums for every artifact, so a client can verify what it downloaded
    # even though this file never says where to download it from.
    artifacts = {}
    for path in sorted(args.dist.iterdir()):
        if path.is_dir() or path.suffix in {".sha256", ".json"}:
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        artifacts[path.name] = {"sha256": digest, "size": path.stat().st_size}

    manifest = {
        "schema": 1,
        "latest": args.version,
        "minimum_supported": MINIMUM_SUPPORTED,
        "summary": args.summary or f"ironwire {args.version}",
        "artifacts": artifacts,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"wrote {args.out} ({len(artifacts)} artifacts)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
