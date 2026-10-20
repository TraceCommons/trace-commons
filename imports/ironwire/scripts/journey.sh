#!/usr/bin/env bash
# Walk the whole user journey against a mock provider.
#
# Not a unit test and not a substitute for one. This runs the *commands a person
# runs*, in the order they run them, and checks the output says something useful
# at each step. It exists because every integration bug found so far in this
# project was found by doing exactly this and none of them were visible from
# inside a unit test.
#
# Run: scripts/journey.sh   (builds in debug; ~1 minute)

set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/ironwire-journey.XXXXXX")
bin="${root}/target/debug/ironwire"
port=18999
mock_port=19000
daemon_pid=""
mock_pid=""

cleanup() {
    [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null
    [ -n "$mock_pid" ] && kill "$mock_pid" 2>/dev/null
    wait 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

export IRONWIRE_HOME="${work}/home"
export CODEX_HOME="${work}/codex"
# `init` now writes to the agents' own config files and installs a service.
# Every path it can touch has to land inside $work, or running this script
# would reconfigure the machine it is testing on.
#
# That means HOME, not CLAUDE_CONFIG_DIR. A CLAUDE_CONFIG_DIR is a Claude login
# of its own, and the default login's settings live in `~/.claude` whatever the
# shell exports — so a CLAUDE_CONFIG_DIR here would leave `init` wiring the real
# `~/.claude/settings.json`. Unset, so one inherited from the developer's shell
# does not turn this journey into a second login's. `~/.claude` exists because
# `init` only wires a Claude Code that has run on the machine.
export HOME="${work}/user"
unset CLAUDE_CONFIG_DIR
claude_dir="${HOME}/.claude"
export ANTHROPIC_API_KEY="sk-ant-journey-test"
export IRONWIRE_ANTHROPIC_BASE_URL="http://127.0.0.1:${mock_port}"
mkdir -p "$IRONWIRE_HOME" "$CODEX_HOME" "$claude_dir"
# A private HOME does not isolate the macOS Keychain. The mock journey must
# explicitly report an absent item and never launch the real security tool.
mkdir -p "${work}/bin"
printf '#!/bin/sh\nexit 44\n' >"${work}/bin/security"
chmod 700 "${work}/bin/security"
export PATH="${work}/bin:${PATH}"

pass=0
fail=0
ok()   { printf '  \033[32mok\033[0m   %s\n' "$1"; pass=$((pass + 1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; printf '       %s\n' "${2:-}" | head -20; fail=$((fail + 1)); }
step() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# Assert that running <cmd> succeeds and its output contains <needle>.
says() {
    local label="$1" needle="$2"; shift 2
    local out
    if ! out=$("$@" 2>&1); then
        bad "$label" "command failed: $*
$out"
        return
    fi
    if grep -qF -- "$needle" <<<"$out"; then
        ok "$label"
    else
        bad "$label" "expected to see '$needle' in:
$out"
    fi
}

# ------------------------------------------------------------------ mock

start_mock() {
    python3 - "$mock_port" <<'PY' &
import http.server, json, socketserver, sys, threading

SSE = (
    b'event: message_start\n'
    b'data: {"type":"message_start","message":{"id":"msg_j","model":"claude-opus-4-6",'
    b'"usage":{"input_tokens":42,"cache_read_input_tokens":900,"output_tokens":1}}}\n\n'
    b'event: content_block_delta\n'
    b'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}\n\n'
    b'event: message_delta\n'
    b'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}\n\n'
    b'event: message_stop\ndata: {"type":"message_stop"}\n\n'
)

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        body = json.dumps({"data": [{"id": "claude-opus-4-6"}]}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("anthropic-ratelimit-unified-limit", "1000")
        self.send_header("anthropic-ratelimit-unified-remaining", "620")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        self.rfile.read(length)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(SSE)))
        self.send_header("anthropic-ratelimit-unified-limit", "1000")
        self.send_header("anthropic-ratelimit-unified-remaining", "620")
        self.end_headers()
        self.wfile.write(SSE)

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

Server(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY
    mock_pid=$!
}

wait_for() {
    local url="$1" tries=50
    while [ $tries -gt 0 ]; do
        curl -fsS -m 1 "$url" >/dev/null 2>&1 && return 0
        sleep 0.2
        tries=$((tries - 1))
    done
    return 1
}

# ------------------------------------------------------------------ journey

[ -x "$bin" ] || { echo "build first: cargo build"; exit 1; }

step "1. A new user runs init"
# `--dry-run` throughout this step: the daemon is not up yet, and the point
# here is what `init` *would* do. It does it for real in step 4.
says "init names capacity it found" "ANTHROPIC_API_KEY" "$bin" init --dry-run
says "init would point Claude Code here" "ANTHROPIC_BASE_URL" "$bin" init --dry-run
says "init would leave the daemon running" "background service" "$bin" init --dry-run
says "init changes nothing in a dry run" "nothing was written" "$bin" init --dry-run

step "2. They start the daemon"
start_mock
wait_for "http://127.0.0.1:${mock_port}/v1/models" || { echo "mock did not start"; exit 1; }
"$bin" serve --port "$port" >"${work}/serve.log" 2>&1 &
daemon_pid=$!
if wait_for "http://127.0.0.1:${port}/_ironwire/health"; then
    ok "the daemon comes up and answers health"
else
    bad "the daemon comes up and answers health" "$(cat "${work}/serve.log")"
    exit 1
fi
grep -q "Point your agents at it" "${work}/serve.log" \
    && ok "startup tells you how to point a client at it" \
    || bad "startup tells you how to point a client at it" "$(cat "${work}/serve.log")"

step "3. doctor, before anything is pointed here"
# The diagnostic deliberately abbreviates HOME with a literal tilde.
# shellcheck disable=SC2088
says "doctor notices no client is pointed here" "~/.claude/settings.json does not point here" \
    "$bin" doctor --port "$port"
says "doctor gives the fix" "ironwire init" "$bin" doctor --port "$port"

step "4. init points the agents at it, for real"
says "init writes a config on request" "Wrote" \
    "$bin" init --port "$port" --write --no-service
says "init does not clobber an existing config" "leaving it alone" \
    "$bin" init --port "$port" --write --no-service
[ -f "${claude_dir}/settings.json" ] \
    && grep -q "127.0.0.1:${port}/anthropic" "${claude_dir}/settings.json" \
    && ok "the setting survives in the file, not just in this shell" \
    || bad "the setting survives in the file, not just in this shell" \
        "$(cat "${claude_dir}/settings.json" 2>&1)"
says "doctor now sees the client" "claude code   pointed here" "$bin" doctor --port "$port"
says "doctor names where it read that from" "settings.json" "$bin" doctor --port "$port"
says "doctor probes the backend" "anthropic-key" "$bin" doctor --port "$port"

step "5. Real traffic"
for i in 1 2 3; do
    curl -fsS -m 10 -X POST "http://127.0.0.1:${port}/anthropic/v1/messages" \
        -H 'content-type: application/json' \
        -d '{"model":"claude-opus-4-6","stream":true,"system":"You are Claude Code","messages":[{"role":"user","content":"turn '"$i"'"}]}' \
        >"${work}/response-${i}.sse" 2>&1
done
grep -q "message_stop" "${work}/response-3.sse" \
    && ok "a request round-trips through the proxy" \
    || bad "a request round-trips through the proxy" "$(cat "${work}/response-3.sse")"

diff <(printf 'event: message_start\n') <(head -1 "${work}/response-1.sse") >/dev/null \
    && ok "the response reaches the client in the provider's own framing" \
    || bad "the response reaches the client in the provider's own framing" "$(head -3 "${work}/response-1.sse")"

step "6. status and log tell them what happened"
says "status shows the backend"          "Anthropic API"   "$bin" status --port "$port"
says "status reports observed capacity"  "% used"          "$bin" status --port "$port"
says "status counts the conversation"    "conversation(s)" "$bin" status --port "$port"
says "status aggregates a balance"       "Balance:"        "$bin" status --port "$port"
says "status measures the session from our own ledger" \
    "measured from IronWire's own ledger" "$bin" status --port "$port"
says "status says how fast the window is going" "tokens/min" "$bin" status --port "$port"
# No completed windows yet and no declared plan, so there is nothing to be a
# percentage *of* — and saying so beats inventing a ceiling to fill the bar.
says "status claims no ceiling it was never given" \
    "nothing to compare against yet" "$bin" status --port "$port"

# Colour has to survive being piped: `says` already captures through a pipe, so
# any escape sequence reaching here would also reach a user's grep or log file.
out=$("$bin" status --port "$port" 2>&1)
if grep -q $'\033' <<<"$out"; then
    bad "piped output carries no escape sequences" "$out"
else
    ok "piped output carries no escape sequences"
fi
out=$("$bin" status --color always --port "$port" 2>&1)
if grep -q $'\033' <<<"$out"; then
    ok "--color always overrides the pipe"
else
    bad "--color always overrides the pipe" "$out"
fi

says "log lists the exchanges"           "claude-opus-4-6" "$bin" log --port "$port"
says "log reports what it cost"          "last 24h"        "$bin" log --port "$port"
says "log shows the tokens the provider reported" "cached" "$bin" log --port "$port"

step "7. The privacy filter, off by default"
says "privacy is off unless asked for" "off" "$bin" privacy status
# The generated config already has a `[privacy]` section, so this edits it
# rather than appending a second one — which TOML rejects, and which is
# exactly the mistake a user following "just add [privacy]" advice would make.
python3 - "${IRONWIRE_HOME}/config.toml" <<'EDIT'
import re, sys
path = sys.argv[1]
text = open(path).read()
text = text.replace('mode = "off"', 'mode = "credentials"')
text = text.replace('named_values = []', 'named_values = ["Acme Holdings"]')
open(path, "w").write(text)
EDIT
says "privacy status reflects the config" "1 named value" "$bin" privacy status
printf 'notes for Acme Holdings\ntoken = "%s%s"\n' 'ghp_' 'abcdefghijklmnopqrstuvwxyz0123456789' >"${work}/sample.txt"
says "privacy check finds both tiers"  "2 match" "$bin" privacy check "${work}/sample.txt"
says "privacy check does not reprint the secret" "…" "$bin" privacy check "${work}/sample.txt"

step "8. Shell integration"
says "env emits POSIX by default"   "export ANTHROPIC_BASE_URL" env SHELL=/bin/bash "$bin" env --port "$port"
says "env emits fish when asked"    "set -gx"                   "$bin" env --shell fish --port "$port"
says "completions generate"         "_ironwire"                 "$bin" completions bash

step "9. Service management"
says "service status names the mechanism" "Service manager" "$bin" service status

step "10. Shutting down"
# With a client holding the event stream open, which is the ordinary state:
# `ironwire watch` in a second terminal, or the menu bar app for the length of
# a login session. `/_ironwire/events` never ends on its own, so a graceful
# shutdown that waits for it waits forever — `systemctl --user stop`, `brew
# services restart` and a plain `kill` all used to hang for as long as anybody
# had a client open. The watchdog is how "hung" is told from "slow": if the
# daemon is still there after ten seconds, it is not going to leave.
"$bin" watch --port "$port" >"$work/watch.out" 2>&1 &
watcher=$!
sleep 1

# Asked of `ps` rather than `wait`, and Z counts as gone: nothing has reaped
# the daemon yet, so a zombie is a process that has exited and a `kill -0`
# would still say it is there.
gone() {
    local state
    state=$(ps -p "$1" -o state= 2>/dev/null | tr -d ' ')
    [ -z "$state" ] || [ "${state#Z}" != "$state" ]
}

kill "$daemon_pid" 2>/dev/null
hung=1
for _ in $(seq 1 20); do
    if gone "$daemon_pid"; then hung=0; break; fi
    sleep 0.5
done
# A daemon that ignored SIGTERM will ignore it for the rest of the run, and
# `wait` on it would hang this script rather than report the failure it just
# found.
[ "$hung" -eq 1 ] && kill -9 "$daemon_pid" 2>/dev/null
wait "$daemon_pid" 2>/dev/null
wait "$watcher" 2>/dev/null

if [ "$hung" -eq 1 ]; then
    bad "a client on the event stream does not hold the daemon open" \
        "the daemon was still running ten seconds after SIGTERM"
else
    ok "a client on the event stream does not hold the daemon open"
fi
# And the client is told, rather than left looking at a socket that died.
if grep -q "closed the stream" "$work/watch.out"; then
    ok "watch says the daemon closed the stream"
else
    bad "watch says the daemon closed the stream" "$(cat "$work/watch.out")"
fi
daemon_pid=""
out=$("$bin" status --port "$port" 2>&1)
if grep -q "not running" <<<"$out"; then
    ok "a stopped daemon says so, rather than a transport error"
else
    bad "a stopped daemon says so, rather than a transport error" "$out"
fi

printf '\n'
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n' "$pass"
else
    printf '\033[31m%d failed\033[0m, %d passed\n' "$fail" "$pass"
    exit 1
fi
