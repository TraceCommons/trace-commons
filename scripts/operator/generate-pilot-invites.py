#!/usr/bin/env python3
"""
Generate Trace Commons pilot invite links and hash-only allowlist entries.

Raw invite codes are printed only to stdout or to --links-out. The allowlist
file receives only canonical subject hashes.
"""

import argparse
import datetime as dt
import hashlib
import json
import os
import secrets
import stat
import sys
import tempfile
from pathlib import Path


ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
DEFAULT_CODE_LENGTH = 16
DEFAULT_MAX_USES = 3


def invite_code(length: int) -> str:
    return "".join(secrets.choice(ALPHABET) for _ in range(length))


def subject_hash(code: str) -> str:
    digest = hashlib.sha256(("invite:" + code).encode("utf-8")).hexdigest()
    return "sha256:" + digest


def now_utc() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def load_allowlist(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as f:
        data = json.load(f)
    if data.get("version") != 1:
        raise SystemExit(f"unsupported allowlist version in {path}: {data.get('version')!r}")
    entries = data.get("entries")
    if not isinstance(entries, list):
        raise SystemExit(f"allowlist entries must be an array: {path}")
    return data


def atomic_write_json(path: Path, data: dict) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    existing = path.exists()
    old_stat = path.stat() if existing else None
    backup = None
    if existing:
        backup = path.with_name(path.name + ".bak." + dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
        backup.write_bytes(path.read_bytes())
        os.chmod(backup, stat.S_IMODE(old_stat.st_mode))
        try:
            os.chown(backup, old_stat.st_uid, old_stat.st_gid)
        except PermissionError:
            pass

    fd, tmp_name = tempfile.mkstemp(prefix=path.name + ".", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, sort_keys=False)
            f.write("\n")
        if old_stat is not None:
            os.chmod(tmp_name, stat.S_IMODE(old_stat.st_mode))
            try:
                os.chown(tmp_name, old_stat.st_uid, old_stat.st_gid)
            except PermissionError:
                pass
        else:
            os.chmod(tmp_name, 0o640)
        os.replace(tmp_name, path)
    except Exception:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass
        raise
    return backup


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("--count", type=int, required=True, help="Number of invite links to generate")
    parser.add_argument("--tenant-id", required=True, help="Tenant ID to place in hash-only allowlist entries")
    parser.add_argument("--issuer-url", default="https://issuer.tracecommons.ai", help="Issuer origin for invite links")
    parser.add_argument("--note-label", default="closed-alpha-batch-1", help="Pseudonymous operator batch label")
    parser.add_argument("--policy-label", default="pilot-2026-05", help="Policy label for newly-created allowlist files")
    parser.add_argument("--max-uses", type=int, default=DEFAULT_MAX_USES, help="Device registrations allowed per invite")
    parser.add_argument("--code-length", type=int, default=DEFAULT_CODE_LENGTH, help="Invite code length")
    parser.add_argument("--allowlist", type=Path, help="Allowlist JSON file to update")
    parser.add_argument("--write", action="store_true", help="Atomically update --allowlist")
    parser.add_argument("--links-out", type=Path, help="Write raw invite links to this local operator file")
    parser.add_argument("--entries-out", type=Path, help="Write generated hash-only entries as JSON")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.count <= 0:
        raise SystemExit("--count must be positive")
    if args.max_uses <= 0:
        raise SystemExit("--max-uses must be positive")
    if args.code_length < 8:
        raise SystemExit("--code-length must be at least 8")
    if args.write and not args.allowlist:
        raise SystemExit("--write requires --allowlist")

    issuer_url = args.issuer_url.rstrip("/")
    generated = []
    seen_codes = set()
    while len(generated) < args.count:
        code = invite_code(args.code_length)
        if code in seen_codes:
            continue
        seen_codes.add(code)
        h = subject_hash(code)
        generated.append(
            {
                "code": code,
                "link": f"{issuer_url}/onboard#{code}",
                "entry": {
                    "subject_hash": h,
                    "tenant_id": args.tenant_id,
                    "note_label": args.note_label,
                    "max_uses": args.max_uses,
                },
            }
        )

    existing_hashes = set()
    allowlist = None
    if args.allowlist and args.allowlist.exists():
        allowlist = load_allowlist(args.allowlist)
        existing_hashes = {entry.get("subject_hash") for entry in allowlist.get("entries", [])}
    elif args.allowlist:
        allowlist = {
            "version": 1,
            "generated_at": now_utc(),
            "policy_label": args.policy_label,
            "entries": [],
        }

    duplicate_hashes = [item["entry"]["subject_hash"] for item in generated if item["entry"]["subject_hash"] in existing_hashes]
    if duplicate_hashes:
        raise SystemExit("generated duplicate hash already present in allowlist; retry the command")

    entries = [item["entry"] for item in generated]
    if args.entries_out:
        args.entries_out.write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8")
        os.chmod(args.entries_out, 0o640)

    if args.write:
        assert allowlist is not None
        before = len(allowlist["entries"])
        allowlist["generated_at"] = now_utc()
        allowlist["entries"].extend(entries)
        backup = atomic_write_json(args.allowlist, allowlist)
        after = len(allowlist["entries"])
        print(f"allowlist_entries_before={before}", file=sys.stderr)
        print(f"allowlist_entries_after={after}", file=sys.stderr)
        if backup:
            print(f"allowlist_backup={backup}", file=sys.stderr)

    links = "\n".join(item["link"] for item in generated) + "\n"
    if args.links_out:
        args.links_out.write_text(links, encoding="utf-8")
        os.chmod(args.links_out, 0o600)
        print(f"links_out={args.links_out}", file=sys.stderr)
    else:
        sys.stdout.write(links)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
