#!/usr/bin/env python3
"""Run cargo-audit using the reviewed advisory exceptions in deny.toml.

Accepts cargo-audit options, e.g. --file tauri-desktop/Cargo.lock --json.
Requires Python 3.11+ (tomllib) and cargo-audit on PATH.
"""

import argparse
from pathlib import Path
import subprocess
import sys

import tomllib

from dependency_policy import verified_glib_exception


def main():
    root = Path(__file__).resolve().parents[2]
    with (root / "deny.toml").open("rb") as policy:
        advisories = tomllib.load(policy)["advisories"]
    command = ["cargo", "audit", "--deny", "unsound"]
    # Parse only the lockfile selector; forward the complete CLI unchanged.
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("-f", "--file", default="Cargo.lock")
    args, _ = parser.parse_known_args()
    if (root / args.file).resolve() == root / "tauri-desktop/Cargo.lock":
        command.extend(["--ignore", verified_glib_exception(root)])
    for entry in advisories.get("ignore", []):
        advisory_id = entry if isinstance(entry, str) else entry["id"]
        command.extend(["--ignore", advisory_id])
    return subprocess.run(command + sys.argv[1:], cwd=root, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())
