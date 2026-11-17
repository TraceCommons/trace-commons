#!/usr/bin/env bash
# Check Flatpak host-daemon mode on a real Linux desktop.
#
# Design: docs/superpowers/specs/2026-11-17-linux-flatpak-host-daemon-design.md
#
# Run as the contributor, in a desktop session, with the CLI installed and the
# Flatpak (ai.tracecommons.Contributor) installed. It starts nothing that
# uploads and changes no settings: it reads the runtime directory, probes the
# host link from the host and from inside the sandbox, and confirms the
# sandbox has no network. Each check prints PASS or FAIL; the exit status is
# the number of failures.
#
#   scripts/flatpak/check-host-link.sh
set -uo pipefail

APP_ID=ai.tracecommons.Contributor
UNIT=trace-commons-contributor.service
failures=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }

: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is not set; run this in a login session}"
RUN_DIR="$XDG_RUNTIME_DIR/trace-commons"
SOCK="$RUN_DIR/daemon.sock"

if systemctl --user is-active --quiet "$UNIT"; then
  pass "host daemon unit is active"
else
  fail "host daemon unit is not active (systemctl --user enable --now $UNIT)"
fi

if [ -d "$RUN_DIR" ]; then
  mode=$(stat -c '%a' "$RUN_DIR")
  owner=$(stat -c '%u' "$RUN_DIR")
  if [ "$mode" = 700 ] && [ "$owner" = "$(id -u)" ]; then
    pass "runtime directory is 0700 and ours"
  else
    fail "runtime directory is mode $mode owner $owner"
  fi
else
  fail "no runtime directory at \$XDG_RUNTIME_DIR/trace-commons"
fi

[ -S "$SOCK" ] && pass "host link socket exists" || fail "no host link socket"
[ -e "$RUN_DIR/watcher.lock" ] && pass "watcher lock exists" || fail "no watcher lock"

probe='import socket,sys,json
s=socket.socket(socket.AF_UNIX); s.settimeout(10); s.connect(sys.argv[1])
s.sendall(b"{\"id\":1,\"method\":\"managed_snapshot\",\"params\":{}}\n")
buf=b""
while not buf.endswith(b"\n"):
    chunk=s.recv(65536)
    if not chunk: break
    buf+=chunk
caps=json.loads(buf)["result"]["capabilities"]
print("daemon_on_host=%s terminal_launch=%s" % (caps.get("daemon_on_host"), caps.get("terminal_launch")))'

if out=$(python3 -c "$probe" "$SOCK" 2>/dev/null); then
  pass "host link answers from the host: $out"
else
  fail "host link does not answer from the host"
fi

if command -v flatpak >/dev/null && flatpak info "$APP_ID" >/dev/null 2>&1; then
  if out=$(flatpak run --command=python3 "$APP_ID" -c "$probe" "$SOCK" 2>/dev/null); then
    pass "host link answers from inside the sandbox: $out"
  else
    fail "host link does not answer from inside the sandbox (python3 may be missing from the runtime; try flatpak run --command=sh and socat)"
  fi
  if flatpak run --command=sh "$APP_ID" -c 'getent hosts tracecommons.ai' >/dev/null 2>&1; then
    fail "the sandbox resolved a host name: it has network after all"
  else
    pass "the sandbox has no network"
  fi
else
  fail "the Flatpak $APP_ID is not installed"
fi

printf '%d failure(s)\n' "$failures"
exit "$failures"
