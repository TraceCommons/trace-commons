#!/usr/bin/env python3
"""List the OS credential-store entries this machine's Trace Commons configs
still reference. READ-ONLY: it deletes nothing and never reads a secret.

Why: until the `test-credential-store` feature, every contributor integration
test run wrote ~76 login-keychain items under the production service
`trace-commons.account.credentials` (account `cloud-v1-<uuid>`) and left them.
A real app credential has exactly the same shape, so a cleanup must delete
only the entries NO installed config points at. This prints that split; the
deletion itself is a separate, deliberate step.

Where references live. The CLI, the Tauri app, the macOS app and the GTK app
all share one state directory -- `TRACE_COMMONS_CONTRIBUTOR_DIR` when set,
otherwise `~/Library/Application Support/trace-commons` on macOS and
`~/.config/trace-commons` on Linux. Inside it, a credential reference is a
JSON object `{"version": 1, "id": "<uuid>"}` stored as `cloud-v<version>-<uuid
without dashes>`: in `device.pk8` (device key record), `account-session.json`,
`daemon-settings.json` (Private AI credentials, service
`trace-commons.near-ai.credentials`) and the two cleanup journals. Sibling
directories named `trace-commons*` (for example a hand-made backup) are
scanned too, since restoring one would bring its references back.

Usage:
  scripts/dev/credential-references.py [STATE_DIR ...]
      Print every referenced storage key and the file that references it.
  scripts/dev/credential-references.py --keychain [STATE_DIR ...]
      macOS: also count login-keychain items under both production services
      (attributes only, via `security dump-keychain` without -d) and split
      them into referenced / unreferenced.
  scripts/dev/credential-references.py --keychain --print-unreferenced
      As above, then print the unreferenced account names, one per line, for
      a later cleanup to consume.
"""

import argparse
import glob
import json
import os
import re
import subprocess
import sys
import uuid

SERVICES = ("trace-commons.account.credentials", "trace-commons.near-ai.credentials")
MAX_FILE_BYTES = 4 * 1024 * 1024


def default_state_dirs():
    dirs = []
    env = os.environ.get("TRACE_COMMONS_CONTRIBUTOR_DIR")
    if env:
        dirs.append(env)
    home = os.path.expanduser("~")
    if sys.platform == "darwin":
        base = os.path.join(home, "Library", "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config")
    dirs.extend(sorted(glob.glob(os.path.join(base, "trace-commons*"))))
    seen, out = set(), []
    for d in dirs:
        real = os.path.realpath(d)
        if os.path.isdir(real) and real not in seen:
            seen.add(real)
            out.append(d)
    return out


def storage_key(obj):
    """`cloud-v<version>-<simple uuid>` for a `{"version", "id"}` object."""
    if not isinstance(obj, dict) or set(obj) != {"version", "id"}:
        return None
    version, ident = obj["version"], obj["id"]
    if not isinstance(version, int) or not isinstance(ident, str):
        return None
    try:
        return f"cloud-v{version}-{uuid.UUID(ident).hex}"
    except ValueError:
        return None


def walk(value):
    key = storage_key(value)
    if key:
        yield key
        return
    if isinstance(value, dict):
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def documents(path):
    try:
        if os.path.getsize(path) > MAX_FILE_BYTES:
            return
        with open(path, "rb") as handle:
            raw = handle.read()
    except OSError:
        return
    if not raw.lstrip().startswith((b"{", b"[")):
        return  # e.g. a legacy DER device key: holds no reference
    try:
        yield json.loads(raw)
        return
    except ValueError:
        pass
    for line in raw.splitlines():
        try:
            yield json.loads(line)
        except ValueError:
            continue


def referenced(state_dirs):
    refs = {}
    for state_dir in state_dirs:
        for root, _dirs, files in os.walk(state_dir):
            for name in files:
                path = os.path.join(root, name)
                if os.path.islink(path):
                    continue
                for doc in documents(path):
                    for key in walk(doc):
                        refs.setdefault(key, set()).add(path)
    return refs


def keychain_accounts():
    """{service: set(account)} from the login keychain's attributes only."""
    if sys.platform != "darwin":
        sys.exit("--keychain is macOS-only")
    keychain = os.path.expanduser("~/Library/Keychains/login.keychain-db")
    dump = subprocess.run(
        ["/usr/bin/security", "dump-keychain", keychain],
        capture_output=True,
        text=True,
        errors="replace",
        check=True,
    ).stdout
    found = {service: set() for service in SERVICES}
    for item in dump.split("keychain: ")[1:]:
        svce = re.search(r'"svce"<blob>="([^"]*)"', item)
        acct = re.search(r'"acct"<blob>="([^"]*)"', item)
        if svce and acct and svce.group(1) in found:
            found[svce.group(1)].add(acct.group(1))
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("state_dirs", nargs="*", help="state directories to scan")
    parser.add_argument("--keychain", action="store_true")
    parser.add_argument("--print-unreferenced", action="store_true")
    args = parser.parse_args()

    state_dirs = args.state_dirs or default_state_dirs()
    print("# state directories scanned:", file=sys.stderr)
    for d in state_dirs:
        print(f"#   {d}", file=sys.stderr)
    refs = referenced(state_dirs)
    for key in sorted(refs):
        for path in sorted(refs[key]):
            print(f"{key}\t{path}")

    if not args.keychain:
        return
    accounts = keychain_accounts()
    unreferenced = []
    for service in SERVICES:
        present = accounts[service]
        live = present & set(refs)
        dead = sorted(present - set(refs))
        unreferenced.extend((service, account) for account in dead)
        print(
            f"# {service}: {len(present)} in login keychain, "
            f"{len(live)} referenced, {len(dead)} unreferenced",
            file=sys.stderr,
        )
    missing = sorted(set(refs) - set().union(*accounts.values()))
    if missing:
        print(
            f"# {len(missing)} referenced key(s) not in the login keychain "
            "(another store, or already gone):",
            file=sys.stderr,
        )
        for key in missing:
            print(f"#   {key}", file=sys.stderr)
    if args.print_unreferenced:
        for service, account in unreferenced:
            print(f"{service}\t{account}")


if __name__ == "__main__":
    main()
