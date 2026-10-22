#!/usr/bin/env bash
# Refuse a committed judgement-service compose read-back that is not hardened.
#
# `phala deploy` never reads app-compose.json. It builds its own manifest,
# and by default it turns public_logs and public_sysinfo on and adds
# DSTACK_AUTHORIZED_KEYS to allowed_envs, with a pre-launch script that
# installs root SSH keys (deploy/witness/README.md). A measurement pinned on
# such a manifest would publish the service's logs and admit a root shell
# that can read digests and claims in memory and call GetKey for the limiter
# root. So every deploy reads the manifest back with
# `phala cvms get <id> --json` and commits its `compose_file` verbatim as
#
#   deploy/judgement/compose-readback.<profile>.json   (production|development)
#
# and this check refuses a committed read-back that fails any rule:
#
#   - public_logs is false and public_sysinfo is false;
#   - allowed_envs is exactly ["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"]
#     (production) or [] (development);
#   - DSTACK_AUTHORIZED_KEYS appears nowhere in the file;
#   - pre_launch_script is absent or empty, or writes no authorized_keys and
#     sets no password.
#
# The file may be the `compose_file` object itself, the whole
# `phala cvms get --json` output, or either with `compose_file` as a JSON
# string; all are read the same way. No read-back exists until the first
# deploy (slice 6 for development, slice 9 for production), so an absent
# file passes. Design: section 9, steps 3 and 4 (#1501 review point 6,
# #1531 review point 3).
#
# Usage:
#   scripts/ci/check-judgement-compose-readback.sh check [ROOT]
#   scripts/ci/check-judgement-compose-readback.sh self-test
set -euo pipefail

READBACK_DIR="deploy/judgement"

check_file() {
  # $1: path, $2: profile. Prints one REFUSING line per failed rule.
  python3 -I - "$1" "$2" <<'PY'
import json
import sys

path, profile = sys.argv[1], sys.argv[2]
expected_envs = {
    "production": ["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"],
    "development": [],
}[profile]
raw = open(path, encoding="utf-8").read()
failures = []


def refuse(message):
    failures.append(message)


try:
    value = json.loads(raw)
except ValueError:
    print(f"REFUSING: {path}: not JSON")
    sys.exit(1)

# Unwrap the whole `phala cvms get --json` output, and a compose_file
# carried as a JSON string.
if isinstance(value, dict) and "compose_file" in value:
    value = value["compose_file"]
if isinstance(value, str):
    try:
        value = json.loads(value)
    except ValueError:
        print(f"REFUSING: {path}: compose_file is a string that is not JSON")
        sys.exit(1)
if not isinstance(value, dict):
    print(f"REFUSING: {path}: compose_file is not an object")
    sys.exit(1)

for flag in ("public_logs", "public_sysinfo"):
    if value.get(flag) is not False:
        refuse(f"{flag} must be false, read back {json.dumps(value.get(flag))}")
if value.get("allowed_envs") != expected_envs:
    refuse(
        f"allowed_envs must be exactly {json.dumps(expected_envs)} for {profile}, "
        f"read back {json.dumps(value.get('allowed_envs'))}"
    )
if "DSTACK_AUTHORIZED_KEYS" in raw:
    refuse("DSTACK_AUTHORIZED_KEYS appears in the read-back")
script = value.get("pre_launch_script") or ""
if not isinstance(script, str):
    refuse("pre_launch_script is not a string")
else:
    lowered = script.lower()
    if "authorized_keys" in lowered:
        refuse("pre_launch_script writes authorized_keys")
    if "chpasswd" in lowered or "passwd" in lowered:
        refuse("pre_launch_script sets a password")

for message in failures:
    print(f"REFUSING: {path}: {message}")
sys.exit(1 if failures else 0)
PY
}

check() {
  local root="${1:-.}" failures=0 file found=0 profile
  cd "$root"
  for file in "$READBACK_DIR"/compose-readback*; do
    [[ -e "$file" ]] || continue
    found=$((found + 1))
    case "$(basename "$file")" in
      compose-readback.production.json) profile=production ;;
      compose-readback.development.json) profile=development ;;
      *)
        echo "REFUSING: $file: a read-back is compose-readback.production.json or compose-readback.development.json" >&2
        failures=$((failures + 1))
        continue
        ;;
    esac
    if ! check_file "$file" "$profile" >&2; then
      failures=$((failures + 1))
    fi
  done
  if ((failures > 0)); then
    echo "$failures judgement compose read-back violation(s)." >&2
    return 1
  fi
  if ((found == 0)); then
    echo "no judgement compose read-back committed yet (none until the first deploy)"
  else
    echo "every committed judgement compose read-back is hardened ($found)"
  fi
}

self_test() {
  local scratch status=0
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN
  mkdir -p "$scratch/$READBACK_DIR"

  if ! (check "$scratch" >/dev/null 2>&1); then
    echo "self-test: a tree with no read-back should pass and did not" >&2
    return 1
  fi

  local production="$scratch/$READBACK_DIR/compose-readback.production.json"
  local development="$scratch/$READBACK_DIR/compose-readback.development.json"
  local good_production='{"manifest_version":2,"name":"judgement","runner":"docker-compose","docker_compose_file":"services: {}","kms_enabled":true,"gateway_enabled":true,"public_logs":false,"public_sysinfo":false,"public_tcbinfo":true,"local_key_provider_enabled":false,"allowed_envs":["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"],"no_instance_id":false,"pre_launch_script":""}'
  local good_development='{"manifest_version":2,"name":"judgement-development","runner":"docker-compose","docker_compose_file":"services: {}","kms_enabled":true,"gateway_enabled":true,"public_logs":false,"public_sysinfo":false,"public_tcbinfo":true,"local_key_provider_enabled":false,"allowed_envs":[],"no_instance_id":false}'
  printf '%s\n' "$good_production" >"$production"
  printf '%s\n' "$good_development" >"$development"
  if ! (check "$scratch" >/dev/null 2>&1); then
    echo "self-test: the hardened fixtures should pass and did not" >&2
    (check "$scratch") || true
    return 1
  fi
  # The whole `phala cvms get --json` output, compose_file as a string.
  python3 -I -c 'import json,sys; print(json.dumps({"id": "cvm", "compose_file": sys.argv[1]}))' \
    "$good_production" >"$production"
  if ! (check "$scratch" >/dev/null 2>&1); then
    echo "self-test: a hardened read-back wrapped in the cvms output should pass and did not" >&2
    return 1
  fi
  printf '%s\n' "$good_production" >"$production"

  # Each case: a description, a target (production|development|extra), and
  # a perl substitution seeding one violation.
  local cases=(
    "public_logs true|production|s/\"public_logs\":false/\"public_logs\":true/"
    "public_sysinfo true|development|s/\"public_sysinfo\":false/\"public_sysinfo\":true/"
    "public_logs absent|production|s/\"public_logs\":false,//"
    "DSTACK_AUTHORIZED_KEYS in allowed_envs|production|s/\"allowed_envs\":\\[/\"allowed_envs\":[\"DSTACK_AUTHORIZED_KEYS\",/"
    "DSTACK_AUTHORIZED_KEYS in the compose text only|development|s/services: \\{\\}/services: {DSTACK_AUTHORIZED_KEYS: x}/"
    "a production read-back without the API key|production|s/\"allowed_envs\":\\[[^]]*\\]/\"allowed_envs\":[]/"
    "a development read-back allowing the API key|development|s/\"allowed_envs\":\\[\\]/\"allowed_envs\":[\"TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY\"]/"
    "a second allowed env|production|s/\"allowed_envs\":\\[/\"allowed_envs\":[\"OTHER\",/"
    "a pre-launch script writing authorized_keys|production|s/\"pre_launch_script\":\"\"/\"pre_launch_script\":\"echo \\\$K > \\/home\\/root\\/.ssh\\/authorized_keys\"/"
    "a pre-launch script setting a password|production|s/\"pre_launch_script\":\"\"/\"pre_launch_script\":\"echo root:x | chpasswd\"/"
    "a read-back that is not JSON|development|s/\\A\\{//"
    "a read-back under another name|extra|"
  )
  local entry desc target program path before
  for entry in "${cases[@]}"; do
    IFS='|' read -r desc target program <<<"$entry"
    case "$target" in
      production) path="$production" ;;
      development) path="$development" ;;
      extra) path="$scratch/$READBACK_DIR/compose-readback.staging.json" ;;
    esac
    if [[ "$target" == extra ]]; then
      printf '%s\n' "$good_development" >"$path"
    else
      before="$(cat "$path")"
      perl -0pi -e "$program" "$path"
      if [[ "$(cat "$path")" == "$before" ]]; then
        echo "self-test: seeding '$desc' changed nothing; the fixture drifted" >&2
        status=1
        continue
      fi
    fi
    if (check "$scratch" >/dev/null 2>&1); then
      echo "self-test: '$desc' was NOT refused" >&2
      status=1
    else
      echo "self-test: refused as expected: $desc"
    fi
    if [[ "$target" == extra ]]; then
      rm -f "$path"
    else
      printf '%s\n' "$before" >"$path"
    fi
  done
  return "$status"
}

case "${1:-}" in
  check) check "${2:-.}" ;;
  self-test) self_test ;;
  *)
    echo "usage: $0 {check [ROOT]|self-test}" >&2
    exit 2
    ;;
esac
