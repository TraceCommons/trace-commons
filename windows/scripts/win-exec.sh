#!/usr/bin/env bash
#
# Run a PowerShell command on the Windows dev box and print its output here.
#
# This is the headless counterpart to RDP. RDP is for watching the app render;
# this is for everything else -- `cargo build`, `dotnet test`, reading a
# compiler error -- which is most of the work and does not want a GUI.
#
# The instance has no external IP, so every connection is tunnelled through
# IAP, which Google IAM authenticates before a packet reaches the host. The
# tunnel is opened per invocation and torn down on exit: a long-lived
# background tunnel is a process to forget about and leak, and the setup cost
# here is a couple of seconds.
#
# Usage:
#   ./win-exec.sh 'cargo --version'
#   ./win-exec.sh 'cd C:\src\trace-commons-server; cargo build -p trace-commons-contributor-ffi --release'
#
set -euo pipefail

PROJECT="${TC_WIN_VM_PROJECT:-tracecommons-pilot-2026}"
ZONE="${TC_WIN_VM_ZONE:-us-central1-a}"
NAME="${TC_WIN_VM_NAME:-tc-win-dev}"
KEY="${TC_WIN_VM_KEY:-$HOME/.ssh/tc-win-dev}"
USER_NAME="${TC_WIN_VM_SSH_USER:-tcdev}"
LOCAL_PORT="${TC_WIN_VM_SSH_PORT:-12222}"

if [ $# -lt 1 ]; then
  echo "usage: win-exec.sh '<powershell command>'" >&2
  exit 1
fi

COMMAND="$*"

cleanup() {
  if [ -n "${TUNNEL_PID:-}" ] && kill -0 "$TUNNEL_PID" 2>/dev/null; then
    kill "$TUNNEL_PID" 2>/dev/null || true
    wait "$TUNNEL_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

gcloud compute start-iap-tunnel "$NAME" 22 \
  --local-host-port="localhost:${LOCAL_PORT}" \
  --project "$PROJECT" \
  --zone "$ZONE" >/dev/null 2>&1 &
TUNNEL_PID=$!

# Wait for the tunnel to accept connections rather than sleeping a fixed
# interval. A fixed sleep is either too short on a cold tunnel or wasted on a
# warm one, and this loop is bounded so a tunnel that never comes up fails
# with a clear message instead of hanging.
for _ in $(seq 1 30); do
  if nc -z localhost "$LOCAL_PORT" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$TUNNEL_PID" 2>/dev/null; then
    echo "IAP tunnel exited before it accepted connections." >&2
    exit 1
  fi
  sleep 1
done

if ! nc -z localhost "$LOCAL_PORT" 2>/dev/null; then
  echo "IAP tunnel did not come up on localhost:${LOCAL_PORT}" >&2
  exit 1
fi

# StrictHostKeyChecking=no with a throwaway known-hosts file: the host key
# changes whenever the box is rebuilt, and the connection's actual security
# boundary is IAP's IAM check plus the key below, not TOFU on a host key we
# would have no independent way to verify anyway.
ssh -i "$KEY" \
    -p "$LOCAL_PORT" \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR \
    "${USER_NAME}@localhost" \
    "$COMMAND"
