#!/usr/bin/env python3
"""Inert Developer ID signature check; never grants packaged-worker launch."""

import argparse
import pathlib
import re
import subprocess
import sys

CODESIGN = "/usr/bin/codesign"
TIMEOUT_SECONDS = 30


def requirement(identifier, team):
    """Trust comes from the caller's reviewed policy, never package metadata."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", identifier):
        raise ValueError("compute-signature-policy-invalid")
    if not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("compute-signature-policy-invalid")
    return (
        f'anchor apple generic and identifier "{identifier}" '
        'and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
        'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists '
        f'and certificate leaf[subject.OU] = "{team}"'
    )


def checked_paths(bundle):
    bundle = pathlib.Path(bundle)
    if not bundle.is_absolute() or not bundle.is_dir():
        raise ValueError("compute-signature-path-invalid")
    # Conservative developer-tool path rules, not a replacement-race defense.
    worker = bundle / "Contents/Helpers/holonear"
    for path in (worker, *worker.parents):
        if path.is_symlink():
            raise ValueError("compute-signature-path-invalid")
    if not worker.is_file():
        raise ValueError("compute-signature-path-invalid")
    return bundle, worker


def verify_signature(path, required):
    try:
        result = subprocess.run(
            [CODESIGN, "--verify", "--strict", "--all-architectures",
             "-R=" + required, str(path)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=TIMEOUT_SECONDS, check=False,
            env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
        )
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError("compute-signature-verifier-unavailable") from None
    if result.returncode != 0:
        raise ValueError("compute-signature-refused")


def verify(bundle, app_identifier, worker_identifier, team):
    # Validate all independent policy before inspecting or invoking a verifier.
    app_requirement = requirement(app_identifier, team)
    worker_requirement = requirement(worker_identifier, team)
    app, worker = checked_paths(bundle)
    if sys.platform != "darwin":
        raise ValueError("compute-signature-platform-unsupported")
    verify_signature(worker, worker_requirement)
    verify_signature(app, app_requirement)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle")
    parser.add_argument("app_identifier")
    parser.add_argument("worker_identifier")
    parser.add_argument("team")
    args = parser.parse_args()
    try:
        verify(args.bundle, args.app_identifier, args.worker_identifier, args.team)
    except ValueError as error:
        # Only our fixed labels; subprocess and filesystem paths are suppressed.
        print(str(error), file=sys.stderr)
        return 1
    except OSError:
        print("compute-signature-read-failed", file=sys.stderr)
        return 1
    print("compute-signatures-verified integrity_verified=false "
          "provenance_verified=false launch_authorized=false")
    return 0


if __name__ == "__main__":
    sys.exit(main())
