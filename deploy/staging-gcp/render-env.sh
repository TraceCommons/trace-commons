#!/usr/bin/env bash
# First, before anything is read: `bash -x` or an inherited xtrace would
# print every secret as it is assigned.
set +x
# Render the staging server's configuration from Secret Manager (staging
# design 3.7). Run on the staging VM, as root, from a checkout of the commit
# being deployed:
#
#   sudo TC_GCP_REGION=<region> TC_LE_EMAIL=<acme email> \
#     deploy/staging-gcp/render-env.sh
#
# Reads every tc-staging-* secret with
# `gcloud secrets versions access latest` under the VM's runtime service
# account, and writes, each file replaced in one step:
#
#   /etc/tracecommons/ingest.env            the pilot's ingest template with
#                                           staging values, then
#                                           deploy/staging-gcp/ingest.env.template
#                                           over it; plus the score-attestation
#                                           keypair
#   /etc/tracecommons/issuer.env            deploy/staging-gcp/issuer.env.template
#   /etc/tracecommons/issuer-signing-v1.pem the issuer's signing key
#   /etc/tracecommons/issuer-signing.pub    its public half
#   /etc/tracecommons/workload-signing.pub  the workload-token public key
#
# each mode 0640 root:tracecommons, written to a temporary file in the same
# directory and renamed into place. And, holding nothing secret:
#
#   <site root>/.well-known/apple-app-site-association   (0644)
#   <site root>/near-ai-pins/                            (0755; the operator
#                                                         writes current.json)
#   <deploy dir>/Caddyfile, trace-commons-ingest.service,
#   trace-commons-upload-claim-issuer.service            for deploy.sh
#
# No secret value is printed, logged or passed on a command line: each is
# read into a variable and reaches its file through envsubst on stdin. A
# missing or empty secret, or one in the wrong form, refuses by name and
# writes nothing. Rotation: add a secret version, rerun this, restart.
#
# Settings (environment):
#   TC_CREDIT_SETTLEMENT_ALL_TENANTS  default 0; explicit 1 enables live internal settlement.
#   TC_CREDIT_SETTLEMENT_POLICY_VERSION required with automatic settlement; approved policy version.
#   See docs/operator/automatic-credit-settlement.md for DB role prerequisites.
#   TC_GCP_REGION       required: the region of the staging KMS key ring.
#   TC_LE_EMAIL         required: the ACME account email for Caddy.
#   TC_GCP_PROJECT      default tracecommons-pilot-2026.
#   TC_PUBLIC_HOST      default staging.tracecommons.ai.
#   TC_KMS_LOCATION     default TC_GCP_REGION.
#   TC_NEAR_AI_EXPECTED_MEASUREMENTS
#                       optional: the line `trace-commons-staging-pins sign`
#                       prints for the ingest (staging design 5.9), value only.
#                       Kept in <out dir>/near-ai-expected-measurements and
#                       read back from there by every later render.
#   TC_NEAR_AI_PAYOUT_MODE
#                       default disabled. dry_run or live also require
#                       TC_NEAR_AI_PAYOUT_CREDENTIAL_ID, _SENDER_ORG, _SOURCE
#                       (payment, grant or staking_farm; no default) and
#                       _ACCOUNT_CEILING_NANO, and read the secret
#                       tc-staging-nearai-reseller-key, which must be an org
#                       credential: oc_ and 64 hex characters. They also
#                       require TC_CREDIT_SETTLEMENT_TABLE_PATH, the absolute
#                       path of the installed settlement table (an existing
#                       file; it carries the payout rate), written as
#                       TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH.
#                       live also reads the secret
#                       tc-staging-payout-control-token (32 or more
#                       characters on one line), the operator's token for
#                       halt and resume, written as
#                       TRACE_COMMONS_PAYOUT_CONTROL_TOKEN; dry_run does not.
#   TC_REFERRAL_PROGRAM_START
#                       optional RFC 3339 time: the referral program opens
#                       (codes can be made and applied), written as
#                       TRACE_COMMONS_REFERRAL_PROGRAM_START. Unset, the
#                       program has not started. Grant trace_referral_resolver
#                       first (docs/operator/referral-resolver-role.md).
#   TC_REFERRAL_PROGRAM_END, TC_REFERRAL_LATE_DAYS, TC_REFERRAL_EARN_DAYS,
#   TC_REFERRAL_BONUS_BPS
#                       optional, each written as TRACE_COMMONS_REFERRAL_*
#                       only when given; unset, the ingest's defaults hold.
#   TC_NEAR_AI_REFERRAL_MODE
#                       default disabled. dry_run needs TC_NEAR_AI_PAYOUT_MODE
#                       dry_run or live; live needs it live, and sends with
#                       the payout credential (one budget and one send window
#                       for both streams). Optional, live only: a dedicated
#                       credential, TC_NEAR_AI_REFERRAL_CREDENTIAL_ID with the
#                       secret tc-staging-nearai-referral-key (an oc_ org
#                       credential). Optional TC_REFERRAL_ACCOUNT_CEILING_NANO.
#                       With a program start or a mode other than disabled,
#                       the render also reads the secret
#                       tc-staging-referral-network-mark-key (base64 of 32 or
#                       more bytes), written as
#                       TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY.
#   TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN, TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX
#                       optional, both or neither: the first run's estimated
#                       NEAR AI credit per accepted trace, in micro-dollars,
#                       written as TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_*.
#                       Unset, the app words its first run in points. The
#                       values are the operator's and are never committed.
#   TC_SCORING_DRIVER   optional: 1 turns on the perplexity-scoring driver,
#                       written as TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED=true
#                       with TRACE_COMMONS_GATE_DRIVER_DATABASE_URL on the
#                       local database as the LOGIN role tc_gate_driver_login.
#                       Reads the secret tc-staging-db-gate-driver-password
#                       (16 or more letters and digits). Unset, nothing is
#                       rendered and the secret is not read.
#   TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA, TC_NOVELTY_UTILITY_SETTLE_FROM
#                       optional: the gate's scoring award, a settling credit.
#                       The delta (a decimal, default the pilot template's 0)
#                       replaces TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA.
#                       Above 0 it also requires the cut-over, an RFC 3339 UTC
#                       time such as 2026-11-01T00:00:00Z, written as
#                       TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM, and renders
#                       TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true,
#                       as the ingest refuses to boot without both.
#   TC_SCORER_PRODUCTION_HOST, TC_SCORER_MODEL
#                       optional, both or neither: score against a NEAR AI
#                       production direct-completions host (staging design
#                       3.9, owner exception 2026-10-24). The host is
#                       https://<model>.completions.near.ai/v1 exactly, the
#                       model its id (Qwen/Qwen3.8-27B). Written as
#                       TRACE_COMMONS_NEAR_AI_BASE_URL and _MODEL, with
#                       TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST=true and
#                       TRACE_COMMONS_NEAR_AI_SCORER_API_KEY from the secret
#                       tc-staging-scorer-production-nearai-key (a production
#                       NEAR AI API key; its account pays for scoring). Every
#                       other NEAR AI setting stays staging. Unset, nothing is
#                       rendered and the secret is not read.
#   TC_CLIENT_UPLOAD_TRUST_MODE
#                       optional shadow or applied; unset stays disabled. Both
#                       require TC_UPLOAD_TRUST_GATE_POLICY_VERSION and
#                       TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION from live staging
#                       decisions, and read tc-staging-account-trust-worker-token
#                       plus tc-staging-db-upload-trust-password for the dedicated
#                       non-owner trust login. The main database URL stays as-is.
#                       Expired/revoked scheduler credentials fail all ingest startup.
#                       Rotate, rerender/restart and verify before revoking the old jti.
#                       Runs the scoped trust scheduler every 300 seconds with
#                       limit 10000. Keeps both admission ledgers off. See
#                       docs/operator/client-upload-allowance.md for readiness.
#   TC_RENDER_OUT_DIR   default /etc/tracecommons.
#   TC_SITE_ROOT        default /var/lib/tracecommons-site.
#   TC_DEPLOY_DIR       default ~/deploy of the user who ran sudo.
#
# --secrets-from-env reads each secret from TC_SECRET_<name, - as _> instead
# of Secret Manager. It exists for scripts/ci/test-deploy-render.sh, which
# also sets TC_RENDER_SKIP_CHOWN=1 so it can run without root.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

refuse() {
  echo "render-env: refusing: $*" >&2
  exit 1
}

secrets_from_env=0
case "${1:-}" in
  "") ;;
  --secrets-from-env) secrets_from_env=1 ;;
  *) echo "usage: $0 [--secrets-from-env]" >&2; exit 2 ;;
esac

TC_GCP_PROJECT="${TC_GCP_PROJECT:-tracecommons-pilot-2026}"
TC_PUBLIC_HOST="${TC_PUBLIC_HOST:-staging.tracecommons.ai}"
TC_GCP_REGION="${TC_GCP_REGION:-}"
TC_LE_EMAIL="${TC_LE_EMAIL:-}"
[ -n "$TC_GCP_REGION" ] || refuse "TC_GCP_REGION is not set"
[ -n "$TC_LE_EMAIL" ] || refuse "TC_LE_EMAIL is not set"
TC_KMS_LOCATION="${TC_KMS_LOCATION:-$TC_GCP_REGION}"
TC_NEAR_AI_PAYOUT_MODE="${TC_NEAR_AI_PAYOUT_MODE:-disabled}"
out_dir="${TC_RENDER_OUT_DIR:-/etc/tracecommons}"
TC_SITE_ROOT="${TC_SITE_ROOT:-/var/lib/tracecommons-site}"
if [ -n "${TC_DEPLOY_DIR:-}" ]; then
  deploy_dir="$TC_DEPLOY_DIR"
elif [ -n "${SUDO_USER:-}" ]; then
  deploy_dir="$(getent passwd "$SUDO_USER" | cut -d: -f6)/deploy"
else
  deploy_dir="$HOME/deploy"
fi
skip_chown="${TC_RENDER_SKIP_CHOWN:-0}"
if [ "$skip_chown" != 1 ] && [ "$(id -u)" != 0 ]; then
  refuse "run as root (sudo); the files are root:tracecommons"
fi
command -v envsubst >/dev/null || refuse "envsubst (gettext-base) is not installed"

# The staging resources (staging design 3.1, S9). Public names, not secrets.
TC_GCS_BUCKET=tc-staging-artifacts-20261021
TC_KMS_KEY_NAME="projects/$TC_GCP_PROJECT/locations/$TC_KMS_LOCATION/keyRings/tc-staging/cryptoKeys/kek-v1"
TC_RUNTIME_SA="tc-staging-runtime@$TC_GCP_PROJECT.iam.gserviceaccount.com"

# A private directory for gcloud's error output, removed on exit with the
# render's working directory.
private="$(mktemp -d)"
work=""
cleanup() {
  rm -rf "$private"
  [ -z "$work" ] || rm -rf "$work"
}
trap cleanup EXIT

# One secret's value, or a refusal naming the secret. Never echoed.
secret() {
  local name="$1" value
  if [ "$secrets_from_env" = 1 ]; then
    local var="TC_SECRET_${name//-/_}"
    value="${!var:-}"
  else
    # gcloud's error output holds no secret value, but it does name the
    # secret's full resource path, so only its error class is repeated.
    local errors class
    errors="$(mktemp "$private/gcloud.XXXXXX")"
    if ! value="$(gcloud secrets versions access latest --secret="$name" \
      --project="$TC_GCP_PROJECT" 2>"$errors")"; then
      class="$(grep -o -E 'PERMISSION_DENIED|NOT_FOUND|UNAUTHENTICATED|FAILED_PRECONDITION|INVALID_ARGUMENT|RESOURCE_EXHAUSTED|UNAVAILABLE' \
        "$errors" | head -n 1 || true)"
      rm -f "$errors"
      refuse "secret $name could not be read (${class:-missing, empty or not readable by the runtime service account})"
    fi
    rm -f "$errors"
  fi
  [ -n "$value" ] || refuse "secret $name is missing or empty"
  printf '%s' "$value"
}

# The shape of a secret that goes into one KEY=value line or a URL, checked
# before it is used: $1 the secret's name, $2 its value, $3 an extended
# regular expression the whole value must match, $4 what that means. A
# newline is refused first, by itself: it would add a line of its own to the
# env file, which no later check could tell from a line of the template.
shaped() {
  case "$2" in
    *$'\n'* | *$'\r'*) refuse "secret $1 holds more than one line" ;;
  esac
  [[ "$2" =~ $3 ]] || refuse "secret $1 is not $4"
}

# A 32-byte key as `openssl rand -hex 32` prints it. SecretsCrypto itself
# needs only 32 characters; hex is what this deployment generates, and
# anything else here is a mistake.
hex_key() {
  shaped "$1" "$2" '^[0-9A-Fa-f]{64}$' "64 hex characters (openssl rand -hex 32)"
}

# A database password goes into a postgres:// URL unencoded, so it may hold
# only characters that need no encoding there.
db_password() {
  shaped "$1" "$2" '^[A-Za-z0-9]{16,}$' "16 or more letters and digits"
}

# The one PEM block of type $2 in $1, or nothing.
pem_block() {
  printf '%s\n' "$1" | awk -v type="$2" '
    $0 == "-----BEGIN " type "-----" { inside = 1; blocks++ }
    inside { print }
    $0 == "-----END " type "-----" { inside = 0 }
    END { if (blocks != 1 || inside) exit 1 }'
}

TC_DB_PASSWORD="$(secret tc-staging-db-app-password)"
db_password tc-staging-db-app-password "$TC_DB_PASSWORD"
TC_LOGIN_RESOLVER_PASSWORD="$(secret tc-staging-db-login-resolver-password)"
db_password tc-staging-db-login-resolver-password "$TC_LOGIN_RESOLVER_PASSWORD"
TC_NEAR_AI_API_KEY="$(secret tc-staging-nearai-inference-key)"
shaped tc-staging-nearai-inference-key "$TC_NEAR_AI_API_KEY" \
  '^[A-Za-z0-9._~+/=:-]+$' "one token of letters, digits and ._~+/=:-"
TC_GATE_MASTER_KEY_HEX="$(secret tc-staging-gate-master-key)"
hex_key tc-staging-gate-master-key "$TC_GATE_MASTER_KEY_HEX"
TC_ARTIFACT_KEY_HEX="$(secret tc-staging-artifact-key)"
hex_key tc-staging-artifact-key "$TC_ARTIFACT_KEY_HEX"
TC_NEAR_ACCOUNT_INDEX_PEPPER="$(secret tc-staging-near-account-index-pepper)"
shaped tc-staging-near-account-index-pepper "$TC_NEAR_ACCOUNT_INDEX_PEPPER" \
  '^[A-Za-z0-9+/]+={0,2}$' "base64 (openssl rand -base64 32)"
pepper_bytes="$(printf '%s' "$TC_NEAR_ACCOUNT_INDEX_PEPPER" | base64 -d 2>/dev/null | wc -c | tr -d ' ')" \
  || pepper_bytes=0
[ "${pepper_bytes:-0}" -ge 32 ] \
  || refuse "secret tc-staging-near-account-index-pepper is not base64 of at least 32 bytes"
TC_NEAR_ACCOUNT_NAME_KEK_KEY="$(secret tc-staging-near-account-name-kek-key)"
hex_key tc-staging-near-account-name-kek-key "$TC_NEAR_ACCOUNT_NAME_KEK_KEY"

issuer_keypair="$(secret tc-staging-issuer-signing-keypair)"
issuer_private="$(pem_block "$issuer_keypair" "PRIVATE KEY")" \
  || refuse "secret tc-staging-issuer-signing-keypair holds no single private key"
issuer_public="$(pem_block "$issuer_keypair" "PUBLIC KEY")" \
  || refuse "secret tc-staging-issuer-signing-keypair holds no single public key"
TC_ISSUER_SIGNING_KID="$(printf '%s\n' "$issuer_keypair" | sed -n 's/^kid: *//p')"
[[ "$TC_ISSUER_SIGNING_KID" =~ ^[0-9A-Za-z-]{8,64}$ ]] \
  || refuse "secret tc-staging-issuer-signing-keypair holds no kid line"
workload_public="$(pem_block "$(secret tc-staging-issuer-workload-public-key)" "PUBLIC KEY")" \
  || refuse "secret tc-staging-issuer-workload-public-key is not one public key"

# The score-attestation keypair, as --generate-attestation-keypair prints
# it: exactly its three settings, nothing else outside the quoted PEMs.
attestation="$(secret tc-staging-ingest-attestation-keypair)"
attestation_keys="$(printf '%s\n' "$attestation" | awk '
  quoted { if (index($0, "\047")) quoted = 0; next }
  /^[[:space:]]*(#|$)/ { next }
  match($0, /^[A-Z_][A-Z0-9_]*=/) {
    print substr($0, 1, RLENGTH - 1)
    rest = substr($0, RLENGTH + 1)
    n = gsub(/\047/, "", rest)
    if (n % 2 == 1) quoted = 1
    next
  }
  { print "?" }' | sort | tr '\n' ' ')"
[ "$attestation_keys" = "TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID " ] \
  || refuse "secret tc-staging-ingest-attestation-keypair is not exactly the three attestation settings"

# Settings appended after the staging overlay, each replacing the line of
# the same key.
extra=""
# The measurement pins are not secret, and they outlive one render: a value
# given here is kept in $pins_file, and a render without one reads it back,
# so rerunning this to rotate a secret never unpins the ingest. Delete the
# file to unpin.
pins_file="$out_dir/near-ai-expected-measurements"
pins=""
if [ -n "${TC_NEAR_AI_EXPECTED_MEASUREMENTS:-}" ]; then
  pins="$TC_NEAR_AI_EXPECTED_MEASUREMENTS"
  pins_from=TC_NEAR_AI_EXPECTED_MEASUREMENTS
elif [ -f "$pins_file" ]; then
  pins="$(cat "$pins_file")"
  pins_from="$pins_file"
  [ -n "$pins" ] || refuse "$pins_file is empty; delete it to unpin"
fi
if [ -n "$pins" ]; then
  [[ "$pins" =~ ^[a-z0-9=,]+$ ]] \
    || refuse "$pins_from is not in the field=<hex>,... form"
  extra+="TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=$pins"$'\n'
fi
first_run_min="${TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN:-}"
first_run_max="${TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX:-}"
if [ -n "$first_run_min" ] || [ -n "$first_run_max" ]; then
  # Shape only; the ingest checks the range itself and refuses to boot on a
  # bad one.
  [[ "$first_run_min" =~ ^[0-9]+$ && "$first_run_max" =~ ^[0-9]+$ ]] \
    || refuse "TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN and _MAX must both be set, in digits"
  extra+="TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN=$first_run_min"$'\n'
  extra+="TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MAX=$first_run_max"$'\n'
fi
# The scoring driver: scores the staging corpus's traces in the background,
# and with the scoring award below it earns each scored trace a settling
# award. Off unless TC_SCORING_DRIVER=1. It reads the whole corpus through
# the narrow trace_gate_driver role, so it needs that role's LOGIN member,
# tc_gate_driver_login, made after the first boot like the login resolver's.
case "${TC_SCORING_DRIVER:-}" in
  "") ;;
  1)
    gate_driver_password="$(secret tc-staging-db-gate-driver-password)"
    db_password tc-staging-db-gate-driver-password "$gate_driver_password"
    extra+="TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED=true"$'\n'
    extra+="TRACE_COMMONS_GATE_DRIVER_DATABASE_URL=postgres://tc_gate_driver_login:$gate_driver_password@127.0.0.1:5432/tracecommons"$'\n'
    ;;
  *) refuse "TC_SCORING_DRIVER must be 1 or unset" ;;
esac
# The scorer on a NEAR AI production direct-completions host (staging design
# 3.9, owner exception 2026-10-24). NEAR AI staging serves no endpoint that
# scores with echo and logprobs, so with both TC_SCORER_PRODUCTION_HOST and
# TC_SCORER_MODEL set the scorer calls the production host, with the
# production key from tc-staging-scorer-production-nearai-key and only that
# key. Every other NEAR AI setting stays staging, and the ingest checks so at
# boot. Unset, nothing is rendered and the secret is not read.
scorer_host="${TC_SCORER_PRODUCTION_HOST:-}"
scorer_model="${TC_SCORER_MODEL:-}"
if [ -n "$scorer_host" ] || [ -n "$scorer_model" ]; then
  [ -n "$scorer_host" ] || refuse "TC_SCORER_PRODUCTION_HOST is not set (TC_SCORER_MODEL is)"
  [ -n "$scorer_model" ] || refuse "TC_SCORER_MODEL is not set (TC_SCORER_PRODUCTION_HOST is)"
  # The shape the ingest admits: https, one DNS label, then
  # .completions.near.ai, then /v1 and nothing else.
  [[ "$scorer_host" =~ ^https://[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.completions\.near\.ai/v1$ ]] \
    || refuse "TC_SCORER_PRODUCTION_HOST is not https://<model>.completions.near.ai/v1"
  [[ "$scorer_model" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] \
    || refuse "TC_SCORER_MODEL is not a model id (letters, digits and ._/-)"
  scorer_key="$(secret tc-staging-scorer-production-nearai-key)"
  shaped tc-staging-scorer-production-nearai-key "$scorer_key" \
    '^[A-Za-z0-9._~+/=:-]+$' "one token of letters, digits and ._~+/=:-"
  extra+="TRACE_COMMONS_NEAR_AI_BASE_URL=$scorer_host"$'\n'
  extra+="TRACE_COMMONS_NEAR_AI_MODEL=$scorer_model"$'\n'
  extra+="TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST=true"$'\n'
  extra+="TRACE_COMMONS_NEAR_AI_SCORER_API_KEY=$scorer_key"$'\n'
fi
# The gate's scoring award. The pilot template sets the delta to 0 and staging
# inherits it; a delta above 0 replaces that line, and then, as the ingest
# refuses to boot otherwise, the production-gate requirement is rendered and a
# cut-over is required (novelty_utility_credit_requires_production_gate,
# novelty_utility_settle_from_missing/_invalid). Checked here so a bad render
# fails at render time.
award_delta="${TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA:-}"
award_settle_from="${TC_NOVELTY_UTILITY_SETTLE_FROM:-}"
if [ -n "$award_settle_from" ]; then
  # RFC 3339 in UTC, as the cut-over is written: 2026-11-01T00:00:00Z.
  [[ "$award_settle_from" =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(\.[0-9]+)?Z$ ]] \
    || refuse "TC_NOVELTY_UTILITY_SETTLE_FROM is not an RFC 3339 UTC time (2026-11-01T00:00:00Z)"
  if [ "$((10#${BASH_REMATCH[2]}))" -lt 1 ] || [ "$((10#${BASH_REMATCH[2]}))" -gt 12 ] \
    || [ "$((10#${BASH_REMATCH[3]}))" -lt 1 ] || [ "$((10#${BASH_REMATCH[3]}))" -gt 31 ] \
    || [ "$((10#${BASH_REMATCH[4]}))" -gt 23 ] || [ "$((10#${BASH_REMATCH[5]}))" -gt 59 ] \
    || [ "$((10#${BASH_REMATCH[6]}))" -gt 59 ]; then
    refuse "TC_NOVELTY_UTILITY_SETTLE_FROM is not an RFC 3339 UTC time (2026-11-01T00:00:00Z)"
  fi
fi
if [ -n "$award_delta" ]; then
  # Shape only; the ingest checks the policy ceiling and refuses to boot over it.
  [[ "$award_delta" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
    || refuse "TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA is not a non-negative decimal number"
  extra+="TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=$award_delta"$'\n'
  if [[ "$award_delta" =~ [1-9] ]]; then
    [ -n "$award_settle_from" ] \
      || refuse "TC_NOVELTY_UTILITY_SETTLE_FROM is not set (TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA is above 0)"
    extra+="TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true"$'\n'
    extra+="TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM=$award_settle_from"$'\n'
  fi
fi
# A deliberate opt-in only: labels without a mode refuse rather than enabling
# evaluation accidentally. Label shape matches the tiered policy parser.
upload_trust_mode="${TC_CLIENT_UPLOAD_TRUST_MODE:-}"
upload_trust_gate="${TC_UPLOAD_TRUST_GATE_POLICY_VERSION:-}"
upload_trust_dedup="${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION:-}"
case "$upload_trust_mode" in
  "")
    [ -z "$upload_trust_gate" ] && [ -z "$upload_trust_dedup" ] \
      || refuse "TC_CLIENT_UPLOAD_TRUST_MODE is required with upload trust labels"
    ;;
  shadow | applied)
    [[ "$upload_trust_gate" =~ ^[A-Za-z0-9._+-]{1,64}$ ]] \
      || refuse "TC_UPLOAD_TRUST_GATE_POLICY_VERSION must be a safe 1-64 character label"
    [[ "$upload_trust_dedup" =~ ^[A-Za-z0-9._+-]{1,64}$ ]] \
      || refuse "TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION must be a safe 1-64 character label"
    # The policy is public configuration; only these two checked labels are
    # substituted. Its labels contain no whitespace, so stripping template
    # whitespace produces one JSON line without a JSON-tool dependency.
    export TC_UPLOAD_TRUST_GATE_POLICY_VERSION="$upload_trust_gate"
    export TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION="$upload_trust_dedup"
    upload_trust_policy="$(envsubst '${TC_UPLOAD_TRUST_GATE_POLICY_VERSION} ${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}' \
      <"$here/client-upload-trust-policy.json.template" | tr -d '[:space:]')"
    upload_trust_password="$(secret tc-staging-db-upload-trust-password)"
    db_password tc-staging-db-upload-trust-password "$upload_trust_password"
    upload_trust_token="$(secret tc-staging-account-trust-worker-token)"
    shaped tc-staging-account-trust-worker-token "$upload_trust_token" \
      '^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$' "one JWT-shaped token"
    if [ "$upload_trust_mode" = shadow ]; then
      extra+="TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_VERSION=staging-earned-upload-v1"$'\n'
      extra+="TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON=$upload_trust_policy"$'\n'
    else
      extra+="TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_VERSION=staging-earned-upload-v1"$'\n'
      extra+="TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON=$upload_trust_policy"$'\n'
    fi
    extra+="TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL=postgres://tc_upload_trust_login:$upload_trust_password@127.0.0.1:5432/tracecommons"$'\n'
    extra+="TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED=true"$'\n'
    extra+="TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS=300"$'\n'
    extra+="TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT=10000"$'\n'
    extra+="TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN=$upload_trust_token"$'\n'
    # Independent client caps must not change staging signup or admission.
    extra+="TRACE_COMMONS_ADMISSION_ENABLED=false"$'\n'
    extra+="TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED=false"$'\n'
    ;;
  *) refuse "TC_CLIENT_UPLOAD_TRUST_MODE must be shadow or applied, or unset" ;;
esac

case "${TC_CREDIT_SETTLEMENT_ALL_TENANTS:-0}" in
  0) ;;
  1)
    settlement_policy="${TC_CREDIT_SETTLEMENT_POLICY_VERSION:-}"
    [[ "$settlement_policy" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] \
      || refuse "TC_CREDIT_SETTLEMENT_POLICY_VERSION is required for automatic settlement"
    extra+="TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED=true"$'\n'
    extra+="TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_SCOPE=all_tenants"$'\n'
    extra+="TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION=$settlement_policy"$'\n'
    extra+="TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN=false"$'\n'
    ;;
  *) refuse "TC_CREDIT_SETTLEMENT_ALL_TENANTS must be 0 or 1" ;;
esac

case "$TC_NEAR_AI_PAYOUT_MODE" in
  disabled) ;;
  dry_run | live)
    for setting in CREDENTIAL_ID SENDER_ORG SOURCE ACCOUNT_CEILING_NANO; do
      var="TC_NEAR_AI_PAYOUT_$setting"
      [[ "${!var:-}" =~ ^[A-Za-z0-9_-]+$ ]] || refuse "$var is not set (payout $TC_NEAR_AI_PAYOUT_MODE)"
    done
    # The credit type the sender org sends from. No default: staging's org
    # holds paid credit (`payment`), and the right one is the operator's call.
    case "$TC_NEAR_AI_PAYOUT_SOURCE" in
      payment | grant | staking_farm) ;;
      *) refuse "TC_NEAR_AI_PAYOUT_SOURCE must be payment, grant or staking_farm" ;;
    esac
    # The rate the pass prices accruals at is the installed settlement
    # table's. Without one the ingest still boots, but the rail has no rate
    # in force: every pass stops with `payout_rate_unavailable` before it
    # accrues or sends, even when an earlier boot stored a rate epoch, and
    # readiness shows the rail unqualified. An enabled rail therefore refuses
    # to render without the table. The file is the operator's, installed
    # before this render; only its path is here.
    table_path="${TC_CREDIT_SETTLEMENT_TABLE_PATH:-}"
    [[ "$table_path" =~ ^/[A-Za-z0-9_./-]+$ ]] \
      || refuse "TC_CREDIT_SETTLEMENT_TABLE_PATH is not an absolute path (payout $TC_NEAR_AI_PAYOUT_MODE)"
    [ -f "$table_path" ] \
      || refuse "TC_CREDIT_SETTLEMENT_TABLE_PATH is not a file (payout $TC_NEAR_AI_PAYOUT_MODE)"
    extra+="TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH=$table_path"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=$TC_NEAR_AI_PAYOUT_MODE"$'\n'
    reseller_key="$(secret tc-staging-nearai-reseller-key)"
    # An org credential, as cloud-api issues it and the ingest parses it:
    # `oc_` and 64 hex. An API key (`sk-...`) is the wrong credential.
    shaped tc-staging-nearai-reseller-key "$reseller_key" \
      '^oc_[0-9A-Fa-f]{64}$' "an org credential (oc_ and 64 hex characters)"
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL=$reseller_key"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID=$TC_NEAR_AI_PAYOUT_CREDENTIAL_ID"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG=$TC_NEAR_AI_PAYOUT_SENDER_ORG"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE=$TC_NEAR_AI_PAYOUT_SOURCE"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO=$TC_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO"$'\n'
    # A live rail must be haltable: the operator's payout control token is
    # the only credential halt and resume accept, and the ingest refuses to
    # boot a live rail without one (payout_control_token_unconfigured).
    # dry_run sends nothing, so it neither needs nor reads it.
    if [ "$TC_NEAR_AI_PAYOUT_MODE" = live ]; then
      control_token="$(secret tc-staging-payout-control-token)"
      shaped tc-staging-payout-control-token "$control_token" \
        '^[A-Za-z0-9._~+/=-]{32,}$' "32 or more characters of letters, digits and ._~+/=- (openssl rand -hex 32)"
      extra+="TRACE_COMMONS_PAYOUT_CONTROL_TOKEN=$control_token"$'\n'
    fi
    ;;
  *) refuse "TC_NEAR_AI_PAYOUT_MODE must be disabled, dry_run or live" ;;
esac

# Referral rewards (docs/superpowers/specs/2026-10-29-referral-rewards-design.md).
# Off unless the operator turns them on: with no TC_REFERRAL_PROGRAM_START
# and TC_NEAR_AI_REFERRAL_MODE disabled, nothing below is rendered and no
# referral secret is read, and the ingest reads the unset settings as off.
# The ingest validates every value at boot and refuses a malformed one; the
# shapes checked here only catch a mistake before the restart.
TC_NEAR_AI_REFERRAL_MODE="${TC_NEAR_AI_REFERRAL_MODE:-disabled}"
case "$TC_NEAR_AI_REFERRAL_MODE" in
  disabled | dry_run | live) ;;
  *) refuse "TC_NEAR_AI_REFERRAL_MODE must be disabled, dry_run or live" ;;
esac
referral_on=0
rfc3339='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$'
if [ -n "${TC_REFERRAL_PROGRAM_START:-}" ]; then
  [[ "$TC_REFERRAL_PROGRAM_START" =~ $rfc3339 ]] \
    || refuse "TC_REFERRAL_PROGRAM_START is not an RFC 3339 time"
  extra+="TRACE_COMMONS_REFERRAL_PROGRAM_START=$TC_REFERRAL_PROGRAM_START"$'\n'
  referral_on=1
fi
if [ -n "${TC_REFERRAL_PROGRAM_END:-}" ]; then
  [[ "$TC_REFERRAL_PROGRAM_END" =~ $rfc3339 ]] \
    || refuse "TC_REFERRAL_PROGRAM_END is not an RFC 3339 time"
  extra+="TRACE_COMMONS_REFERRAL_PROGRAM_END=$TC_REFERRAL_PROGRAM_END"$'\n'
fi
# Optional tuning, rendered only when given; unset, the ingest's defaults
# hold (late days 30, earning days 92, bonus 1000 bps). The ingest refuses an
# earning period under 92 days.
for name in TRACE_COMMONS_REFERRAL_LATE_DAYS TRACE_COMMONS_REFERRAL_EARN_DAYS \
  TRACE_COMMONS_REFERRAL_BONUS_BPS; do
  var="TC_${name#TRACE_COMMONS_}"
  [ -n "${!var:-}" ] || continue
  [[ "${!var}" =~ ^[0-9]{1,5}$ ]] || refuse "$var is not a whole number"
  extra+="$name=${!var}"$'\n'
done
if [ "$TC_NEAR_AI_REFERRAL_MODE" != disabled ]; then
  # The bonus stream sends on the NEAR AI rail's links: it needs the rail
  # on, and live needs the rail live (the ingest refuses either at boot,
  # near_ai_referral_config_missing).
  [ "$TC_NEAR_AI_PAYOUT_MODE" != disabled ] \
    || refuse "TC_NEAR_AI_REFERRAL_MODE $TC_NEAR_AI_REFERRAL_MODE needs TC_NEAR_AI_PAYOUT_MODE dry_run or live"
  if [ "$TC_NEAR_AI_REFERRAL_MODE" = live ] && [ "$TC_NEAR_AI_PAYOUT_MODE" != live ]; then
    refuse "TC_NEAR_AI_REFERRAL_MODE live needs TC_NEAR_AI_PAYOUT_MODE live"
  fi
  extra+="TRACE_COMMONS_NEAR_AI_REFERRAL_MODE=$TC_NEAR_AI_REFERRAL_MODE"$'\n'
  if [ -n "${TC_REFERRAL_ACCOUNT_CEILING_NANO:-}" ]; then
    [[ "$TC_REFERRAL_ACCOUNT_CEILING_NANO" =~ ^[1-9][0-9]{0,38}$ ]] \
      || refuse "TC_REFERRAL_ACCOUNT_CEILING_NANO is not a positive whole number"
    extra+="TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO=$TC_REFERRAL_ACCOUNT_CEILING_NANO"$'\n'
  fi
  # A live stream sends with the payout credential unless a dedicated one
  # is given (owner decision, 2026-10-29): then both streams share one
  # lifetime budget and one send window. A dedicated credential is set by
  # TC_NEAR_AI_REFERRAL_CREDENTIAL_ID and the secret
  # tc-staging-nearai-referral-key together. Half of the payout pair beside
  # half of another is refused, as the ingest refuses it. dry_run sends
  # nothing and reads neither.
  if [ "$TC_NEAR_AI_REFERRAL_MODE" = live ] && [ -n "${TC_NEAR_AI_REFERRAL_CREDENTIAL_ID:-}" ]; then
    uuid='^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$'
    [[ "$TC_NEAR_AI_REFERRAL_CREDENTIAL_ID" =~ $uuid ]] \
      || refuse "TC_NEAR_AI_REFERRAL_CREDENTIAL_ID is not a UUID"
    referral_key="$(secret tc-staging-nearai-referral-key)"
    shaped tc-staging-nearai-referral-key "$referral_key" \
      '^oc_[0-9A-Fa-f]{64}$' "an org credential (oc_ and 64 hex characters)"
    same_key=0
    same_id=0
    [ "$referral_key" != "$reseller_key" ] || same_key=1
    [ "$(printf '%s' "$TC_NEAR_AI_REFERRAL_CREDENTIAL_ID" | tr 'A-F' 'a-f')" \
      != "$(printf '%s' "$TC_NEAR_AI_PAYOUT_CREDENTIAL_ID" | tr 'A-F' 'a-f')" ] || same_id=1
    [ "$same_key" = "$same_id" ] \
      || refuse "the referral credential and its id must both be the payout pair or both differ"
    extra+="TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL=$referral_key"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL_ID=$TC_NEAR_AI_REFERRAL_CREDENTIAL_ID"$'\n'
  fi
  referral_on=1
fi
# Configured referrals need the network mark key (the ingest refuses to boot
# without it, referral_network_mark_key_missing): base64 of at least 32
# bytes, as `openssl rand -base64 32` prints it.
if [ "$referral_on" = 1 ]; then
  mark_key="$(secret tc-staging-referral-network-mark-key)"
  shaped tc-staging-referral-network-mark-key "$mark_key" \
    '^[A-Za-z0-9+/]{43,}={0,2}$' "base64 of 32 or more bytes (openssl rand -base64 32)"
  extra+="TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY=$mark_key"$'\n'
fi

export TC_PUBLIC_HOST TC_GCP_PROJECT TC_GCP_REGION TC_LE_EMAIL TC_GCS_BUCKET \
  TC_KMS_KEY_NAME TC_RUNTIME_SA TC_DB_PASSWORD \
  TC_LOGIN_RESOLVER_PASSWORD TC_NEAR_AI_API_KEY TC_GATE_MASTER_KEY_HEX \
  TC_ARTIFACT_KEY_HEX TC_NEAR_ACCOUNT_INDEX_PEPPER TC_NEAR_ACCOUNT_NAME_KEK_KEY \
  TC_ISSUER_SIGNING_KID TC_SITE_ROOT
# Placeholders the pilot template names that staging does not use. Both
# render empty: the PII-backstop driver URL is commented out there, and the
# community tenant line is removed by the staging overlay.
export TC_PII_BACKSTOP_DRIVER_PASSWORD="" TC_TENANT_ID=""

# Only these names are substituted, so nothing else in a template that looks
# like a variable is touched. envsubst takes them literally, unexpanded.
# shellcheck disable=SC2016
ingest_vars='${TC_DB_PASSWORD} ${TC_GCP_PROJECT} ${TC_GCP_REGION} ${TC_KMS_KEY_NAME} ${TC_GCS_BUCKET} ${TC_RUNTIME_SA} ${TC_PUBLIC_HOST} ${TC_GATE_MASTER_KEY_HEX} ${TC_NEAR_AI_API_KEY} ${TC_ARTIFACT_KEY_HEX} ${TC_LOGIN_RESOLVER_PASSWORD} ${TC_PII_BACKSTOP_DRIVER_PASSWORD} ${TC_TENANT_ID} ${TC_NEAR_ACCOUNT_INDEX_PEPPER} ${TC_NEAR_ACCOUNT_NAME_KEK_KEY}'
# shellcheck disable=SC2016
issuer_vars='${TC_PUBLIC_HOST} ${TC_ISSUER_SIGNING_KID} ${TC_DB_PASSWORD}'
# shellcheck disable=SC2016
caddy_vars='${TC_PUBLIC_HOST} ${TC_LE_EMAIL} ${TC_SITE_ROOT}'

# Everything is built in a private directory beside its destination, then
# renamed, so a refusal at any point leaves the live files as they were.
mkdir -p "$out_dir"
work="$(mktemp -d "$out_dir/.render-env.XXXXXX")"
umask 027

# The pilot render with the staging overlay applied: an overlay key replaces
# the pilot's line, an empty overlay value removes it, and the last overlay
# line for a key wins. Comments and blank lines are dropped.
awk '
    FNR == 1 { file++ }
    /^[[:space:]]*(#|$)/ { next }
    !match($0, /^[A-Za-z_][A-Za-z0-9_]*=/) { bad = "an unparsed line in a template"; exit 1 }
    {
      key = substr($0, 1, RLENGTH - 1)
      if (file == 1) { base[++nbase] = key; line[key] = $0; next }
      if (!(key in over)) order[++nover] = key
      over[key] = $0
    }
    END {
      if (bad != "") { print "render-env: refusing: " bad > "/dev/stderr"; exit 1 }
      for (i = 1; i <= nbase; i++) {
        key = base[i]
        if (key in seen) { print "render-env: refusing: a duplicate key in the pilot template" > "/dev/stderr"; exit 1 }
        seen[key] = 1
        if (!(key in over)) print line[key]
      }
      for (i = 1; i <= nover; i++) {
        key = order[i]
        if (over[key] != key "=") print over[key]
      }
    }' \
    <(envsubst "$ingest_vars" <"$repo/deploy/pilot-gcp/ingest.env.template") \
    <(envsubst "$ingest_vars" <"$here/ingest.env.template"; printf '%s' "$extra") \
    >"$work/ingest.merged"
{
  echo "# Rendered by deploy/staging-gcp/render-env.sh. Do not edit; rerun it."
  cat "$work/ingest.merged"
  printf '%s\n' "$attestation"
} >"$work/ingest.env"

{
  echo "# Rendered by deploy/staging-gcp/render-env.sh. Do not edit; rerun it."
  envsubst "$issuer_vars" <"$here/issuer.env.template" | grep -v -E '^[[:space:]]*(#|$)'
} >"$work/issuer.env"

printf '%s\n' "$issuer_private" >"$work/issuer-signing-v1.pem"
printf '%s\n' "$issuer_public" >"$work/issuer-signing.pub"
printf '%s\n' "$workload_public" >"$work/workload-signing.pub"

# Nothing left unrendered, and no key assigned twice.
for f in ingest.env issuer.env; do
  # shellcheck disable=SC2016
  if grep -v -E '^[[:space:]]*#' "$work/$f" | grep -q -F '${'; then
    refuse "$f has an unrendered placeholder"
  fi
  dups="$(grep -E '^[A-Z_][A-Z0-9_]*=' "$work/$f" | sed 's/=.*//' | sort | uniq -d)"
  [ -z "$dups" ] || refuse "$f assigns a key more than once: $dups"
done

{
  envsubst "$caddy_vars" <"$repo/deploy/pilot-gcp/Caddyfile.template"
  envsubst "$caddy_vars" <"$here/Caddyfile.site.template"
} >"$work/Caddyfile"

for f in ingest.env issuer.env issuer-signing-v1.pem issuer-signing.pub workload-signing.pub; do
  chmod 640 "$work/$f"
  [ "$skip_chown" = 1 ] || chown root:tracecommons "$work/$f"
done
# Each file is replaced in one step (a rename); the set is not. A failure
# between two renames leaves some files new and some old, and a rerun puts
# them in step.
for f in ingest.env issuer.env issuer-signing-v1.pem issuer-signing.pub workload-signing.pub; do
  mv -f "$work/$f" "$out_dir/$f"
done
if [ -n "$pins" ]; then
  printf '%s\n' "$pins" >"$work/near-ai-expected-measurements"
  chmod 644 "$work/near-ai-expected-measurements"
  mv -f "$work/near-ai-expected-measurements" "$pins_file"
fi
echo "render-env: wrote ingest.env, issuer.env and three key files to $out_dir"
if [ -n "$pins" ]; then
  echo "render-env: pinned the NEAR AI measurements from $pins_from"
else
  echo "render-env: no NEAR AI measurements pinned yet (bring-up step 8)"
fi

# The site's static files and the operator's deploy directory.
install -d -m 0755 "$TC_SITE_ROOT" "$TC_SITE_ROOT/.well-known" "$TC_SITE_ROOT/near-ai-pins"
install -m 0644 "$here/apple-app-site-association" \
  "$TC_SITE_ROOT/.well-known/apple-app-site-association"
install -d -m 0755 "$deploy_dir"
install -m 0644 "$work/Caddyfile" "$deploy_dir/Caddyfile"
for unit in trace-commons-ingest.service trace-commons-upload-claim-issuer.service; do
  install -m 0644 "$repo/deploy/pilot-gcp/systemd/$unit" "$deploy_dir/$unit"
done
if [ "$skip_chown" != 1 ] && [ -n "${SUDO_USER:-}" ] && [ -z "${TC_DEPLOY_DIR:-}" ]; then
  chown -R "$SUDO_USER" "$deploy_dir"
fi
echo "render-env: staged the site files under $TC_SITE_ROOT and the Caddyfile and units under $deploy_dir"
