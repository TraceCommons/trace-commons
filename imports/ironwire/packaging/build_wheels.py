#!/usr/bin/env python3
"""Build one wheel per platform from the release artifacts.

The wheel is a delivery vehicle, not a build: it carries the binary that was
already compiled and a console script that execs it. No maturin, no compiler,
no source distribution — an `sdist` here would promise a from-source build that
does not exist, and pip would try it on any platform we missed.

pip is the natural channel for Aider users, who install their agent that way.

Run locally:
    python packaging/build_wheels.py --version 0.1.0 --artifacts <dir> --out dist
"""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import tarfile
import zipfile
from pathlib import Path

# Rust target → the platform tag pip matches against.
#
# manylinux_2_28 rather than a newer tag: it is what the GitHub runner's glibc
# produces and it covers every distro anyone runs a coding agent on. macOS 11 is
# the floor for arm64 and the oldest thing Rust's darwin targets support.
TARGETS = [
    ("aarch64-apple-darwin", "macosx_11_0_arm64", ".tar.gz", "ironwire"),
    ("x86_64-apple-darwin", "macosx_10_12_x86_64", ".tar.gz", "ironwire"),
    ("x86_64-unknown-linux-gnu", "manylinux_2_28_x86_64", ".tar.gz", "ironwire"),
    ("aarch64-unknown-linux-gnu", "manylinux_2_28_aarch64", ".tar.gz", "ironwire"),
    ("x86_64-pc-windows-msvc", "win_amd64", ".zip", "ironwire.exe"),
]

DESCRIPTION = (
    "Localhost router for coding agents — aggregate the subscriptions, "
    "keys and credits you already have"
)


def _extract(archive: Path, member: str) -> bytes:
    """Pull one file out of a .tar.gz or .zip."""
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as zf:
            return zf.read(member)
    with tarfile.open(archive, "r:gz") as tf:
        handle = tf.extractfile(member)
        if handle is None:
            raise KeyError(f"{member} not found in {archive}")
        return handle.read()


def _record_hash(data: bytes) -> tuple[str, int]:
    digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
    return f"sha256={digest.decode()}", len(data)


def build_wheel(version: str, tag: str, binary: bytes, binary_name: str, out: Path) -> Path:
    dist = f"ironwire-{version}"
    wheel_name = f"ironwire-{version}-py3-none-{tag}.whl"
    data_dir = f"{dist}.data/scripts"
    dist_info = f"{dist}.dist-info"

    metadata = f"""Metadata-Version: 2.1
Name: ironwire
Version: {version}
Summary: {DESCRIPTION}
Home-page: https://github.com/nearai/ironwire
License: MIT OR Apache-2.0
Classifier: Development Status :: 3 - Alpha
Classifier: Environment :: Console
Classifier: Intended Audience :: Developers
Classifier: License :: OSI Approved :: MIT License
Classifier: License :: OSI Approved :: Apache Software License
Classifier: Topic :: Software Development
Requires-Python: >=3.8
Description-Content-Type: text/markdown

# IronWire

{DESCRIPTION}.

This wheel contains a prebuilt `ironwire` binary. Nothing is compiled at
install time.

    pip install ironwire
    ironwire connect claude
    ironwire serve

IronWire binds `127.0.0.1` only and keeps its state under `~/.ironwire`.
No subscription is used until you enable it explicitly.

Full documentation: https://github.com/nearai/ironwire
"""

    wheel_meta = f"""Wheel-Version: 1.0
Generator: ironwire-packaging
Root-Is-Purelib: false
Tag: py3-none-{tag}
"""

    entries: list[tuple[str, bytes]] = [
        (f"{data_dir}/{binary_name}", binary),
        (f"{dist_info}/METADATA", metadata.encode()),
        (f"{dist_info}/WHEEL", wheel_meta.encode()),
    ]

    out.mkdir(parents=True, exist_ok=True)
    path = out / wheel_name

    record_rows = []
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            # The binary must arrive executable. pip preserves the mode from
            # the zip entry, and a wheel that installs a non-executable binary
            # fails with "permission denied" at the least helpful moment.
            info.external_attr = (0o755 << 16) if name.startswith(data_dir) else (0o644 << 16)
            zf.writestr(info, data)
            digest, size = _record_hash(data)
            record_rows.append((name, digest, size))

        record = io.StringIO()
        writer = csv.writer(record, lineterminator="\n")
        for row in record_rows:
            writer.writerow(row)
        writer.writerow((f"{dist_info}/RECORD", "", ""))
        info = zipfile.ZipInfo(f"{dist_info}/RECORD", date_time=(1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o644 << 16
        zf.writestr(info, record.getvalue())

    return path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    built = 0
    for target, tag, ext, binary_name in TARGETS:
        archive = args.artifacts / f"ironwire-{target}{ext}"
        if not archive.exists():
            # Loud, not silent: a missing target means users on that platform
            # get "no matching distribution", which reads like the project does
            # not support them at all.
            print(f"warning: no artifact for {target}; skipping")
            continue
        binary = _extract(archive, binary_name)
        path = build_wheel(args.version, tag, binary, binary_name, args.out)
        print(f"  {path.name}  ({len(binary) // 1024} KB)")
        built += 1

    if built == 0:
        raise SystemExit("no platform artifacts found — nothing to build")
    print(f"built {built} wheels in {args.out}/")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
