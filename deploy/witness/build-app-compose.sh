#!/usr/bin/env bash
# Regenerate app-compose.json from docker-compose.yml, and print the SHA-256 of
# the result.
#
# WHY THIS EXISTS. dstack's application manifest embeds the compose file as a
# JSON *string*, so there are two copies of it and only one of them is
# deployed. Editing docker-compose.yml and forgetting this step deploys the old
# configuration -- with a measurement that matches the old configuration, so
# nothing downstream notices. Run this after every edit and commit both files.
#
# WHAT THE HASH IS, AND IS NOT. The value printed is the SHA-256 of the bytes
# of app-compose.json as written here. dstack derives `compose_hash` from the
# manifest it stores, and MRCONFIGID (config-id v1) is `01` followed by that
# hash and fifteen zero bytes. Those are two statements about two different
# artifacts, and this script can only make the first one. Before pinning
# anything, compare this value against `tcb_info.compose_hash` reported by a
# running instance. If they differ, the deployment path canonicalised the
# manifest somewhere and the hash to pin is the instance's, not this one's.
# Nobody on this project has run that comparison against a live agent yet.

#
# `--check` regenerates into a temporary file and exits non-zero if it differs
# from the committed one, without touching it. That is the form to run in CI or
# before a deploy: it answers "is the manifest I am about to upload the one this
# compose file describes", which is the question the drift above turns on.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose="${here}/docker-compose.yml"
manifest="${here}/app-compose.json"

check_only=false
if [[ "${1:-}" == "--check" ]]; then
  check_only=true
  manifest="$(mktemp)"
  trap 'rm -f "${manifest}"' EXIT
elif [[ $# -gt 0 ]]; then
  echo "usage: $(basename "$0") [--check]" >&2
  exit 2
fi

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

jq -S -n --rawfile compose "${compose}" '{
  manifest_version: 2,
  name: "trace-commons-witness",
  runner: "docker-compose",
  docker_compose_file: $compose,

  # The KMS derives the app signing key. This is what makes the signing address
  # survive an image upgrade, and it is why the upgrade story in the README
  # works at all: the key comes from a stable app id, not from a measurement.
  #
  # WHICH KMS is not in this file -- it is a deploy-time flag, --kms-id --
  # but it is not a deploy-time DETAIL. See "Which KMS, and why that is not a
  # detail" in README.md. In short: the signing address derives from the KMS
  # and the app id together, so moving KMS later rotates that address and
  # breaks every client that pinned it, exactly as a guest-API surface change
  # does. Pick once, record why, and treat a change as a key rotation.
  kms_enabled: true,

  # A KMS-derived key and a local key provider are alternatives, not a pair.
  # The local provider seals to the host TPM, which would tie the signing
  # address to one machine and lose the property above.
  local_key_provider_enabled: false,

  # dstack-gateway terminates TLS and proxies to the container port. The
  # witness itself serves plaintext HTTP and has no TLS of its own.
  gateway_enabled: true,

  # THE WITNESS SEES RAW TRANSCRIPTS. Container logs are the most direct way
  # for one to leave, and dstack will serve them publicly if asked to. Do not
  # set this true on a deployment carrying real traffic, for debugging or
  # otherwise.
  public_logs: false,

  # Host and process detail. Nothing here needs it and it is a free
  # fingerprint.
  public_sysinfo: false,

  # Measurements only -- no content. This is the one that should be public:
  # it is how an operator reads mrtd and compose_hash without shelling into
  # the guest, and the measurement is a value we publish on purpose.
  public_tcbinfo: true,

  # One injectable value, and it is a credential rather than a setting.
  #
  # The rule this list started with was that NOTHING is injectable: every
  # setting the witness reads is in the measured compose, so the configuration
  # is part of the enclave identity, and an allowed env var is a knob outside
  # the measurement. That rule still holds for everything that decides what
  # this enclave DOES.
  #
  # `full-pipeline` with an external classifier needs an API key, and the two
  # ways of supplying it are not equal. This repository is public and
  # app-compose.json is committed and uploaded, so a key written into the
  # compose is a published key. Injecting it encrypted at deploy time keeps it
  # out of the manifest and out of git.
  #
  # What makes that safe is what is NOT injectable beside it:
  # TRACE_NEAR_AI_PRIVACY_BASE_URL and TRACE_NEAR_AI_PRIVACY_MODEL are pinned
  # in the compose and therefore measured. The destination and the model are
  # part of the enclave identity; only the credential presented to that fixed
  # destination is injected. A different key cannot redirect a transcript
  # somewhere else -- it can only change which account is billed.
  #
  # Adding any FURTHER name here needs the same argument made explicitly. A
  # name that changes behaviour rather than authenticating to a measured
  # destination reopens the hole this list exists to close.
  allowed_envs: ["TRACE_NEAR_AI_PRIVACY_API_KEY"],

  # Leaving the per-deployment random instance-id in place. It is what makes
  # RTMR3 differ between two instances of identical code, and therefore
  # unpinnable -- see the README. dstack offers `no_instance_id: true` to
  # remove it; we have not evaluated what else that changes, so it stays off
  # rather than being switched on for the convenience of one register.
  no_instance_id: false,

  pre_launch_script: ""
}' > "${manifest}"

digest() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

if [[ "${check_only}" == true ]]; then
  if ! diff -u "${here}/app-compose.json" "${manifest}" >/dev/null 2>&1; then
    echo "app-compose.json is stale: it does not match docker-compose.yml" >&2
    echo "run $(basename "$0") and commit the result" >&2
    diff -u "${here}/app-compose.json" "${manifest}" >&2 || true
    exit 1
  fi
  echo "app-compose.json matches docker-compose.yml"
  echo "sha256 $(digest "${manifest}")"
  exit 0
fi

echo "wrote ${manifest}"
echo "sha256 $(digest "${manifest}")"
