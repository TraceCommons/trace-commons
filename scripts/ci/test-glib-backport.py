#!/usr/bin/env python3
"""Run the GLib iterator regression with optimized code in a temporary crate.

The harness avoids making the vendored dependency a workspace member or
writing Cargo.lock/target files into the provenance-checked vendor tree.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[2]
    vendor = root / "tauri-desktop/vendor/glib"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument(
        "--source", type=Path, default=vendor,
        help="GLib source to test; use an unpatched copy for regression reproduction",
    )
    args = parser.parse_args()
    source = args.source.resolve()
    regression = vendor / "tests/rustsec_2024_0429.rs"
    env = dict(os.environ, RUSTFLAGS="-D warnings", CARGO_INCREMENTAL="0")
    with tempfile.TemporaryDirectory(prefix="tc-glib-backport-") as directory:
        manifest = Path(directory) / "Cargo.toml"
        manifest.write_text(
            '[package]\nname = "trace-glib-backport-regression"\n'
            'version = "0.0.0"\nedition = "2021"\n\n[workspace]\n\n'
            '[dependencies]\nglib = { path = ' + json.dumps(str(source)) + ' }\n\n'
            '[[test]]\nname = "rustsec_2024_0429"\npath = '
            + json.dumps(str(regression)) + '\n',
            encoding="utf-8",
        )
        command = [
            "cargo", "test", "--manifest-path", str(manifest),
            "--release", "--test", "rustsec_2024_0429",
        ]
        if args.offline:
            command.append("--offline")
        return subprocess.run(command, cwd=root, env=env, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
