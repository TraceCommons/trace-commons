#!/usr/bin/env python3
"""Generate a Tauri-only advisory policy after verifying its GLib backport."""

import argparse
import hashlib
from pathlib import Path
import tomllib

GLIB_ADVISORY = "RUSTSEC-2024-0429"
# Covers every vendored file, including manifest, source, license and provenance.
GLIB_TREE_SHA256 = "b6d19497af9f16f9bb312a3d005078a0bb94e0f4511847a3cf6b21e9792cfeef"


def tree_sha256(directory):
    digest = hashlib.sha256()
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"vendored GLib contains a symlink: {path}")
        if path.is_file():
            relative = path.relative_to(directory).as_posix().encode()
            data = path.read_bytes()
            digest.update(len(relative).to_bytes(8, "big"))
            digest.update(relative)
            digest.update(len(data).to_bytes(8, "big"))
            digest.update(data)
    return digest.hexdigest()


def verified_glib_exception(root):
    vendor = root / "tauri-desktop/vendor/glib"
    manifest = tomllib.loads((root / "tauri-desktop/Cargo.toml").read_text())
    patch = manifest.get("patch", {}).get("crates-io", {}).get("glib", {})
    if patch != {"path": "vendor/glib"}:
        raise ValueError("Tauri must use the reviewed vendored GLib patch")
    lock = tomllib.loads((root / "tauri-desktop/Cargo.lock").read_text())
    packages = [p for p in lock["package"] if p["name"] == "glib"]
    if len(packages) != 1 or packages[0]["version"] != "0.18.5+tc.1" or "source" in packages[0]:
        raise ValueError("Tauri lockfile must resolve only the patched local GLib 0.18.5+tc.1")
    if tree_sha256(vendor) != GLIB_TREE_SHA256:
        raise ValueError("vendored GLib differs from the reviewed security backport")
    return GLIB_ADVISORY


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    advisory = verified_glib_exception(root)
    policy = (root / "deny.toml").read_text()
    # Reuse the root policy verbatim, adding one verified, Tauri-only entry.
    entry = (f'    {{ id = "{advisory}", reason = "GLib 0.18.5 is locally patched; '
             'scripts/ci/dependency_policy.py verifies the full vendored tree and lockfile before this policy is generated." },\n')
    marker = "ignore = [\n"
    if policy.count(marker) != 1:
        raise ValueError("expected exactly one root advisory exception list")
    policy = policy.replace(marker, marker + entry, 1)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(policy)


if __name__ == "__main__":
    main()
