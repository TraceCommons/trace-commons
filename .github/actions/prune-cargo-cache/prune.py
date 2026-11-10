#!/usr/bin/env python3
"""Delete a cargo cache family's entries from older Cargo.lock generations.

See action.yml beside this file for why. Standard library only.

    prune.py --key <primary key> [--dry-run]
    prune.py --self-test
"""
import argparse
import datetime
import json
import os
import re
import sys
import subprocess

HASH = re.compile(r"^(?P<prefix>.+-)(?P<hash>[0-9a-f]{64})$")


def parse_time(text):
    """GitHub timestamps, with or without fractional seconds, as UTC."""
    text = text.replace("Z", "+00:00")
    m = re.match(r"^(.*T\d\d:\d\d:\d\d)(\.\d+)?(\+00:00)$", text)
    if not m:
        raise ValueError(f"unexpected timestamp {text!r}")
    frac = (m.group(2) or ".0")[:7]  # microseconds at most
    return datetime.datetime.fromisoformat(m.group(1) + frac + m.group(3))


def select(entries, key, run_started_at):
    """Return (deletions, reason).

    entries: dicts with id, key, created_at (main-branch caches whose key
    starts with this family's prefix). Deletes an entry only when all hold:
    it is this family's (prefix + a 64-hex lockfile hash, nothing longer),
    it is not `key`, `key` itself exists, and it was created before this
    run started. The last rule keeps an older-lockfile run that finishes
    late from deleting the newer generation a later run already saved.
    """
    m = HASH.match(key)
    if not m:
        return [], f"key {key!r} does not end in a 64-hex Cargo.lock hash; refusing"
    prefix = m.group("prefix")
    family = [e for e in entries
              if e["key"].startswith(prefix) and HASH.match(e["key"])
              and HASH.match(e["key"]).group("prefix") == prefix]
    if not any(e["key"] == key for e in family):
        return [], f"{key} is not in the cache list yet; deleting nothing"
    started = parse_time(run_started_at)
    out = [e for e in family
           if e["key"] != key and parse_time(e["created_at"]) < started]
    return out, f"{len(out)} older-generation entr{'y' if len(out) == 1 else 'ies'} of {prefix}*"


def gh(*args):
    """`gh api`, which the hosted runners ship and which authenticates from
    GH_TOKEN; returns its stdout."""
    out = subprocess.run(["gh", "api", *args], check=True, capture_output=True, text=True)
    return out.stdout


def run(key, dry_run):
    repo = os.environ["GITHUB_REPOSITORY"]
    run_id = os.environ["GITHUB_RUN_ID"]
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
    m = HASH.match(key)
    if not m:
        print(f"::warning::{key!r} does not end in a lockfile hash; not pruning")
        return 0
    prefix = m.group("prefix")
    started = gh(f"repos/{repo}/actions/runs/{run_id}/attempts/{attempt}",
                 "--jq", ".run_started_at").strip()
    listing = gh("-X", "GET", f"repos/{repo}/actions/caches", "--paginate",
                 "-f", f"key={prefix}", "-f", "ref=refs/heads/main", "-F", "per_page=100",
                 "--jq", ".actions_caches[] | {id, key, created_at, size_in_bytes}")
    entries = [json.loads(line) for line in listing.splitlines() if line.strip()]
    for e in entries:
        print(f"  listed {e['key']} created {e['created_at']} "
              f"({e.get('size_in_bytes', 0) / 2**30:.2f} GiB)")
    doomed, reason = select(entries, key, started)
    print(f"run started {started}; {reason}")
    for e in doomed:
        verb = "would delete" if dry_run else "deleting"
        print(f"  {verb} {e['key']} (id {e['id']}, created {e['created_at']})")
        if not dry_run:
            gh("-X", "DELETE", f"repos/{repo}/actions/caches/{e['id']}")
    return 0


def self_test():
    h = lambda c: c * 64  # noqa: E731
    new, old, older = f"Linux-cargo-x-{h('a')}", f"Linux-cargo-x-{h('b')}", f"Linux-cargo-x-{h('c')}"
    e = lambda k, t, i=1: {"id": i, "key": k, "created_at": t}  # noqa: E731
    fails = []

    def check(name, got, want):
        keys = sorted(x["key"] for x in got[0])
        if keys != sorted(want):
            fails.append(f"{name}: got {keys}, want {sorted(want)} ({got[1]})")

    base = [e(new, "2026-11-10T01:00:00.123Z"), e(old, "2026-11-09T20:00:00Z"),
            e(older, "2026-11-08T20:00:00Z")]
    check("deletes older generations", select(base, new, "2026-11-10T00:50:00Z"), [old, older])
    check("never the key just saved", select(base, new, "2026-11-11T00:00:00Z"), [old, older])
    check("nothing when the saved key is not listed",
          select(base[1:], new, "2026-11-10T00:50:00Z"), [])
    # An older-lockfile run that started before the newer key was saved,
    # finishing after it, re-saved its own (evicted) key: it must not delete
    # the newer generation.
    late = [e(new, "2026-11-10T01:00:00Z"), e(old, "2026-11-10T01:05:00Z")]
    check("a late older run spares the newer key", select(late, old, "2026-11-10T00:40:00Z"), [])
    # Another family sharing the prefix string is not this family.
    other = base + [e(f"Linux-cargo-x-extra-{h('d')}", "2026-11-01T00:00:00Z"),
                    e(f"Linux-cargo-x-{h('e')}-suffix", "2026-11-01T00:00:00Z")]
    check("other families untouched", select(other, new, "2026-11-10T00:50:00Z"), [old, older])
    check("a key without a hash refuses", select(base, "Linux-cargo-x-", "2026-11-10T00:50:00Z"), [])
    if fails:
        print("prune self-test FAILED:\n  " + "\n  ".join(fails))
        return 1
    print("prune self-test passed (6 cases)")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--key")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.key:
        ap.error("--key is required")
    return run(a.key, a.dry_run)


if __name__ == "__main__":
    sys.exit(main())
