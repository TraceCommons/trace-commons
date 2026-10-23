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
#                       and _ACCOUNT_CEILING_NANO, and read the secret
#                       tc-staging-nearai-reseller-key.
#   TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN, TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX
#                       optional, both or neither: the first run's estimated
#                       NEAR AI credit per accepted trace, in micro-dollars,
#                       written as TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_*.
#                       Unset, the app words its first run in points. The
#                       values are the operator's and are never committed.
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
TC_WEBAUTHN_RP_ID="$TC_PUBLIC_HOST"

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
case "$TC_NEAR_AI_PAYOUT_MODE" in
  disabled) ;;
  dry_run | live)
    for setting in CREDENTIAL_ID SENDER_ORG SOURCE ACCOUNT_CEILING_NANO; do
      var="TC_NEAR_AI_PAYOUT_$setting"
      [[ "${!var:-}" =~ ^[A-Za-z0-9_-]+$ ]] || refuse "$var is not set (payout $TC_NEAR_AI_PAYOUT_MODE)"
    done
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=$TC_NEAR_AI_PAYOUT_MODE"$'\n'
    reseller_key="$(secret tc-staging-nearai-reseller-key)"
    shaped tc-staging-nearai-reseller-key "$reseller_key" \
      '^[A-Za-z0-9._~+/=:-]+$' "one token of letters, digits and ._~+/=:-"
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL=$reseller_key"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID=$TC_NEAR_AI_PAYOUT_CREDENTIAL_ID"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG=$TC_NEAR_AI_PAYOUT_SENDER_ORG"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE=$TC_NEAR_AI_PAYOUT_SOURCE"$'\n'
    extra+="TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO=$TC_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO"$'\n'
    ;;
  *) refuse "TC_NEAR_AI_PAYOUT_MODE must be disabled, dry_run or live" ;;
esac

export TC_PUBLIC_HOST TC_GCP_PROJECT TC_GCP_REGION TC_LE_EMAIL TC_GCS_BUCKET \
  TC_KMS_KEY_NAME TC_RUNTIME_SA TC_WEBAUTHN_RP_ID TC_DB_PASSWORD \
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
ingest_vars='${TC_DB_PASSWORD} ${TC_GCP_PROJECT} ${TC_GCP_REGION} ${TC_KMS_KEY_NAME} ${TC_GCS_BUCKET} ${TC_RUNTIME_SA} ${TC_PUBLIC_HOST} ${TC_WEBAUTHN_RP_ID} ${TC_GATE_MASTER_KEY_HEX} ${TC_NEAR_AI_API_KEY} ${TC_ARTIFACT_KEY_HEX} ${TC_LOGIN_RESOLVER_PASSWORD} ${TC_PII_BACKSTOP_DRIVER_PASSWORD} ${TC_TENANT_ID} ${TC_NEAR_ACCOUNT_INDEX_PEPPER} ${TC_NEAR_ACCOUNT_NAME_KEK_KEY}'
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
