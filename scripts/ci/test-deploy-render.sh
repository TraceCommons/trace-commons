#!/usr/bin/env bash
# The pilot and staging server deploy files, rendered with dummy values and
# run against stubs. No network, no gcloud, no root (staging design 3.6-3.8,
# plan Task 11).
#
# This holds:
#
# - the pilot templates render byte-identical to the fixtures under
#   scripts/ci/tests/fixtures/deploy-pilot, which were rendered from the
#   templates as they stood before the staging parameters were added;
# - deploy/pilot-gcp/deploy.sh run with no staging parameter issues exactly
#   the commands it issued before (fixture deploy.sh.trace), and refuses an
#   ingest whose /health reports a staging build;
# - deploy.sh with the staging parameters skips the Cloud SQL proxy, orders
#   both services after the local PostgreSQL, labels them, and refuses an
#   ingest whose /health does not report a staging build;
# - deploy/staging-gcp/render-env.sh, fed dummy secrets from the environment,
#   renders an ingest env with every staging setting, no pilot bucket, key
#   ring, host or database and no duplicate key, an issuer env, the key
#   files, and a Caddyfile serving the staging site;
# - render-env.sh refuses a missing or empty secret and writes nothing;
# - both Caddyfiles pass `caddy adapt` where caddy is installed.
#
# Seconds.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
fixtures="$repo/scripts/ci/tests/fixtures/deploy-pilot"
failures=0
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

pass() {
  echo "ok: $*"
}

envsubst_bin="$(command -v envsubst || true)"
if [ -z "$envsubst_bin" ]; then
  echo "FAIL: envsubst (gettext) is required" >&2
  exit 1
fi

# The dummy values, as NAME=value lines, passed to envsubst under env -i so
# nothing from the caller's environment leaks into a render.
dummy_env=()
while IFS= read -r line; do
  [ -n "$line" ] && dummy_env+=("$line")
done <"$fixtures/dummy-values.txt"

render_pilot() {
  env -i "${dummy_env[@]}" "$envsubst_bin" <"$1"
}

# 1. The pilot templates render as they did.
for pair in \
  "ingest.env:deploy/pilot-gcp/ingest.env.template" \
  "issuer.env:deploy/pilot-gcp/issuer.env.template" \
  "Caddyfile:deploy/pilot-gcp/Caddyfile.template" \
  "cloud-sql-proxy.service:deploy/pilot-gcp/systemd/cloud-sql-proxy.service"; do
  name="${pair%%:*}"
  template="${pair#*:}"
  render_pilot "$repo/$template" >"$scratch/pilot-$name"
  # Fixtures carry .rendered: the repository ignores *.env.
  if cmp -s "$scratch/pilot-$name" "$fixtures/$name.rendered"; then
    pass "pilot $name renders byte-identical"
  else
    fail "pilot $name render differs from the fixture"
    diff "$fixtures/$name.rendered" "$scratch/pilot-$name" >&2 || true
  fi
done

# 2. deploy.sh against stubs. Every privileged or networked command is a stub
#    that appends its argv to a trace; curl answers /health with
#    $FAKE_HEALTH. HOME is the scratch tree and is written as ~ in the trace,
#    so the trace does not depend on where the test runs.
stubs="$scratch/stubs"
mkdir -p "$stubs"
for tool in sudo systemctl sleep date; do
  cat >"$stubs/$tool" <<STUB
#!/usr/bin/env bash
printf '%s\n' "$tool \$*" | sed "s#\$HOME#~#g" >>"\$TRACE"
case "$tool" in
  date) echo 20260101T000000Z ;;
  sudo)
    # Keep a copy of every unit or drop-in installed, so the test can read
    # what was rendered.
    if [ "\$1" = install ]; then
      for last; do :; done
      src="\${@: -2:1}"
      case "\$src" in *.service|*.conf) [ -f "\$src" ] && cp "\$src" "\$INSTALLED/\$(basename "\$last" | sed 's#/#_#g')-\$(basename "\$src")" ;; esac
    fi
    # A staging deploy checks the rendered env files exist.
    [ "\$1" = test ] && exit 0
    ;;
esac
exit 0
STUB
  chmod +x "$stubs/$tool"
done
cat >"$stubs/pg_isready" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "pg_isready $*" >>"$TRACE"
exit "${FAKE_PG_READY:-0}"
STUB
chmod +x "$stubs/pg_isready"
cat >"$stubs/curl" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "curl $*" >>"$TRACE"
for arg; do
  case "$arg" in
    */health) printf '%s' "$FAKE_HEALTH"; exit 0 ;;
  esac
done
echo '{}'
STUB
chmod +x "$stubs/curl"

run_deploy() {
  # $1: trace file; $2: fake /health body; the rest: NAME=value overrides.
  local trace="$1" health="$2"
  shift 2
  : >"$trace"
  mkdir -p "$scratch/home/deploy" "$scratch/installed"
  # What an operator stages before a deploy: the unit files, as in the repo.
  cp "$repo"/deploy/pilot-gcp/systemd/*.service "$scratch/home/deploy/"
  env -i PATH="$stubs:/usr/bin:/bin" HOME="$scratch/home" TRACE="$trace" \
    INSTALLED="$scratch/installed" FAKE_HEALTH="$health" \
    FAKE_PG_READY="${FAKE_PG_READY:-0}" \
    TC_PUBLIC_HOST=pilot.example.invalid "$@" \
    bash "$repo/deploy/pilot-gcp/deploy.sh" >"$trace.out" 2>"$trace.err"
}

pilot_health='{"status":"ok","near_ai_staging":false}'
run_deploy "$scratch/pilot.trace" "$pilot_health" || true
# To regenerate the fixture after a deliberate change to the pilot path:
# TC_DEPLOY_TRACE_OUT=scripts/ci/tests/fixtures/deploy-pilot/deploy.sh.trace
if [ -n "${TC_DEPLOY_TRACE_OUT:-}" ]; then
  cp "$scratch/pilot.trace" "$TC_DEPLOY_TRACE_OUT"
fi
if [ -s "$scratch/pilot.trace" ] \
  && cmp -s "$scratch/pilot.trace" "$fixtures/deploy.sh.trace"; then
  pass "pilot deploy.sh issues the same commands as before"
else
  fail "pilot deploy.sh trace differs from the fixture"
  diff "$fixtures/deploy.sh.trace" "$scratch/pilot.trace" >&2 || true
  cat "$scratch/pilot.trace.err" >&2 || true
fi
if grep -qx 'PILOT DEPLOY OK' "$scratch/pilot.trace.out"; then
  pass "pilot deploy.sh closes with PILOT DEPLOY OK"
else
  fail "pilot deploy.sh did not close with PILOT DEPLOY OK"
fi

# An ingest from before the field existed reports no near_ai_staging; the
# pilot accepts it.
if run_deploy "$scratch/old.trace" '{"status":"ok"}'; then
  pass "pilot deploy.sh accepts a /health without near_ai_staging"
else
  fail "pilot deploy.sh refused a /health without near_ai_staging"
fi

if run_deploy "$scratch/wrong.trace" '{"status":"ok","near_ai_staging":true}'; then
  fail "pilot deploy.sh accepted an ingest reporting a staging build"
elif grep -q 'near_ai_staging' "$scratch/wrong.trace.err" \
  && ! grep -q 'DEPLOY OK' "$scratch/wrong.trace.out"; then
  pass "pilot deploy.sh refuses an ingest reporting a staging build"
  # Refusing is not enough: the wrong binary is already running. With no
  # earlier binary to go back to (none in this test), it is stopped.
  if grep -qx 'sudo systemctl stop trace-commons-ingest.service' "$scratch/wrong.trace"; then
    pass "pilot deploy.sh stops an ingest from the other environment"
  else
    fail "pilot deploy.sh leaves an ingest from the other environment running"
  fi
else
  fail "pilot deploy.sh failed for some other reason on a staging /health"
  cat "$scratch/wrong.trace.err" >&2
fi

staging_args=(TC_DB_MODE=local TC_DEPLOY_LABEL=STAGING TC_EXPECT_STAGING=1
  TC_ENV_INSTALL=0 TC_BINARY_DIR=/opt/tracecommons/staged
  TC_PUBLIC_HOST=staging.example.invalid)
staging_health='{"status":"ok","near_ai_staging":true}'
rm -rf "$scratch/installed"
if run_deploy "$scratch/staging.trace" "$staging_health" "${staging_args[@]}"; then
  pass "staging deploy.sh runs with a staging /health"
else
  fail "staging deploy.sh failed"
  cat "$scratch/staging.trace.err" >&2
fi
if grep -q 'cloud-sql-proxy' "$scratch/staging.trace"; then
  fail "staging deploy.sh touches the Cloud SQL proxy"
else
  pass "staging deploy.sh never touches the Cloud SQL proxy"
fi
# Ubuntu's postgresql.service is a placeholder that stays active when the
# cluster under it has failed, so local mode asks the server itself.
if grep -qx 'pg_isready -h 127.0.0.1 -p 5432' "$scratch/staging.trace"; then
  pass "staging deploy.sh asks the local PostgreSQL whether it accepts connections"
else
  fail "staging deploy.sh does not run pg_isready against 127.0.0.1:5432"
fi
if FAKE_PG_READY=2 run_deploy "$scratch/staging-nopg.trace" "$staging_health" "${staging_args[@]}"; then
  fail "staging deploy.sh carried on with PostgreSQL not accepting connections"
elif grep -q 'trace-commons-upload-claim-issuer.service' <(grep 'enable' "$scratch/staging-nopg.trace"); then
  fail "staging deploy.sh started a service with PostgreSQL not accepting connections"
else
  pass "staging deploy.sh stops before any service when PostgreSQL is not accepting connections"
fi
if grep -qx 'STAGING DEPLOY OK' "$scratch/staging.trace.out"; then
  pass "staging deploy.sh closes with STAGING DEPLOY OK"
else
  fail "staging deploy.sh did not close with STAGING DEPLOY OK"
fi
if grep -q 'ingest.env /etc/tracecommons/ingest.env' "$scratch/staging.trace"; then
  fail "staging deploy.sh overwrote the rendered env files"
else
  pass "staging deploy.sh leaves the rendered env files in place"
fi
if grep -q '/opt/tracecommons/staged/trace-commons-ingest' "$scratch/staging.trace"; then
  pass "staging deploy.sh installs binaries from TC_BINARY_DIR"
else
  fail "staging deploy.sh did not install from TC_BINARY_DIR"
fi
for unit in trace-commons-ingest.service trace-commons-upload-claim-issuer.service; do
  installed="$(find "$scratch/installed" -name "*-$unit" 2>/dev/null | head -n 1 || true)"
  if [ -z "$installed" ]; then
    fail "staging deploy.sh installed no $unit"
    continue
  fi
  if grep -q 'cloud-sql-proxy' "$installed"; then
    fail "staging $unit still depends on the Cloud SQL proxy"
  elif ! grep -q '^After=.*postgresql.service' "$installed"; then
    fail "staging $unit is not ordered after postgresql.service"
  elif ! grep -q '(TraceCommons staging)' "$installed"; then
    fail "staging $unit is not labelled staging"
  else
    pass "staging $unit runs after the local PostgreSQL, labelled staging"
  fi
  # Every other line is the pilot's: the staging unit is the pilot unit
  # with the database dependency and the label swapped, not a fork.
  if diff <(grep -v -E '^(Description|After|Wants|Requires)=' "$repo/deploy/pilot-gcp/systemd/$unit") \
    <(grep -v -E '^(Description|After|Wants|Requires)=' "$installed") >/dev/null; then
    pass "staging $unit differs from the pilot's only in its label and dependencies"
  else
    fail "staging $unit differs from the pilot's beyond its label and dependencies"
  fi
done

if run_deploy "$scratch/staging-wrong.trace" "$pilot_health" "${staging_args[@]}"; then
  fail "staging deploy.sh accepted an ingest that is not a staging build"
elif grep -q 'near_ai_staging' "$scratch/staging-wrong.trace.err"; then
  pass "staging deploy.sh refuses an ingest that is not a staging build"
  if grep -qx 'sudo systemctl stop trace-commons-ingest.service' "$scratch/staging-wrong.trace"; then
    pass "staging deploy.sh stops an ingest that is not a staging build"
  else
    fail "staging deploy.sh leaves an ingest that is not a staging build running"
  fi
else
  fail "staging deploy.sh failed for some other reason on a pilot /health"
fi
if run_deploy "$scratch/staging-old.trace" '{"status":"ok"}' "${staging_args[@]}"; then
  fail "staging deploy.sh accepted a /health with no near_ai_staging"
else
  pass "staging deploy.sh refuses a /health with no near_ai_staging"
fi
if run_deploy "$scratch/badmode.trace" "$staging_health" TC_DB_MODE=sqlite; then
  fail "deploy.sh accepted an unknown TC_DB_MODE"
else
  pass "deploy.sh refuses an unknown TC_DB_MODE"
fi

# 2b. pull-and-install.sh against the same stubs and a stub gcloud. With
#     TC_EXPECT_STAGING=1 it reads the restarted ingest's /health and, when it
#     is not a staging build, puts the previous binary back. Unset, as on the
#     pilot, it issues no health check at all.
pstubs="$scratch/pull-stubs"
mkdir -p "$pstubs"
cat >"$pstubs/gcloud" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "gcloud $*" >>"$TRACE"
case "$1 $2" in
  "storage cat") bin="$(basename "$(dirname "$3")")"; echo "gs://bucket/binaries/$bin/abc1234/$bin" ;;
  "storage cp")
    case "$3" in *.sha256) exit 1 ;; esac
    echo binary >"$4" ;;
esac
STUB
chmod +x "$pstubs/gcloud"
run_pull() {
  # $1: trace file; $2: fake /health body; the rest: NAME=value overrides.
  local trace="$1" health="$2"
  shift 2
  : >"$trace"
  env -i PATH="$pstubs:$stubs:/usr/bin:/bin" HOME="$scratch/home" TRACE="$trace" \
    INSTALLED="$scratch/installed" FAKE_HEALTH="$health" "$@" \
    bash "$repo/deploy/pilot-gcp/pull-and-install.sh" ingest abc1234 \
    >"$trace.out" 2>"$trace.err"
}
rollback='sudo install -o root -g root -m 0755 /opt/tracecommons/bin/trace-commons-ingest.bak-20260101T000000Z /opt/tracecommons/bin/trace-commons-ingest'
if run_pull "$scratch/pull-pilot.trace" "$pilot_health"; then
  if grep -q '^curl ' "$scratch/pull-pilot.trace"; then
    fail "pull-and-install.sh without TC_EXPECT_STAGING checks /health"
  else
    pass "pull-and-install.sh without TC_EXPECT_STAGING runs as before, with no health check"
  fi
else
  fail "pull-and-install.sh failed without TC_EXPECT_STAGING"
  cat "$scratch/pull-pilot.trace.err" >&2
fi
if run_pull "$scratch/pull-staging.trace" "$staging_health" TC_EXPECT_STAGING=1; then
  if grep -q '/health' "$scratch/pull-staging.trace" && ! grep -qxF "$rollback" "$scratch/pull-staging.trace"; then
    pass "pull-and-install.sh with TC_EXPECT_STAGING=1 keeps a staging build"
  else
    fail "pull-and-install.sh with TC_EXPECT_STAGING=1 did not check, or rolled back, a staging build"
  fi
else
  fail "pull-and-install.sh with TC_EXPECT_STAGING=1 refused a staging build"
  cat "$scratch/pull-staging.trace.err" >&2
fi
for health in "$pilot_health" '{"status":"ok"}' ''; do
  if run_pull "$scratch/pull-wrong.trace" "$health" TC_EXPECT_STAGING=1; then
    fail "pull-and-install.sh with TC_EXPECT_STAGING=1 accepted /health '$health'"
  elif grep -qxF "$rollback" "$scratch/pull-wrong.trace" \
    && [ "$(grep -c '^sudo systemctl restart trace-commons-ingest$' "$scratch/pull-wrong.trace")" = 2 ] \
    && grep -q 'near_ai_staging' "$scratch/pull-wrong.trace.err"; then
    pass "pull-and-install.sh with TC_EXPECT_STAGING=1 rolls back on /health '$health'"
  else
    fail "pull-and-install.sh with TC_EXPECT_STAGING=1 did not roll back on /health '$health'"
    cat "$scratch/pull-wrong.trace" "$scratch/pull-wrong.trace.err" >&2
  fi
done
if run_pull "$scratch/pull-bad.trace" "$staging_health" TC_EXPECT_STAGING=yes; then
  fail "pull-and-install.sh accepted TC_EXPECT_STAGING=yes"
else
  pass "pull-and-install.sh refuses a TC_EXPECT_STAGING other than 1"
fi

# 3. render-env.sh with dummy secrets from the environment.
render="$repo/deploy/staging-gcp/render-env.sh"
[ -f "$render" ] || fail "deploy/staging-gcp/render-env.sh does not exist"

# A refusal counts only when render-env.sh says so by name; a crash or a
# missing script is not a refusal.
refused() {
  grep -q '^render-env: refusing: ' "$scratch/$1.err"
}
out="$scratch/etc"
site="$scratch/site"
deploy_dir="$scratch/deploy"

# Synthetic key material only, made here and never kept.
pem_private=$'-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMY\n-----END PRIVATE KEY-----'
pem_public=$'-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEADUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDUMMYDU=\n-----END PUBLIC KEY-----'
issuer_keypair="$pem_private"$'\n'"$pem_public"$'\nkid: 00000000-0000-4000-8000-000000000001'
attestation=$'# Attestation signing key for trace-commons-ingest.\n\nTRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID=00000000-0000-4000-8000-000000000002\n\nTRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM=\''"$pem_private"$'\n\'\nTRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM=\''"$pem_public"$'\n\''

# Hex keys in the form `openssl rand -hex 32` prints, each distinct and
# recognisable so the printed-secret check below can look for it.
hex_gate="$(printf 'da%.0s' $(seq 32))"
hex_artifact="$(printf 'db%.0s' $(seq 32))"
hex_name_kek="$(printf 'dc%.0s' $(seq 32))"
secret_env=(
  TC_SECRET_tc_staging_nearai_inference_key=dummy-staging-near-ai-key
  TC_SECRET_tc_staging_db_app_password=dummystagingdbpassword
  TC_SECRET_tc_staging_db_login_resolver_password=dummystagingresolverpassword
  "TC_SECRET_tc_staging_gate_master_key=$hex_gate"
  "TC_SECRET_tc_staging_artifact_key=$hex_artifact"
  TC_SECRET_tc_staging_near_account_index_pepper=ZHVtbXktc3RhZ2luZy1wZXBwZXItMDEyMzQ1Njc4OWFiY2RlZg==
  "TC_SECRET_tc_staging_near_account_name_kek_key=$hex_name_kek"
  "TC_SECRET_tc_staging_issuer_signing_keypair=$issuer_keypair"
  "TC_SECRET_tc_staging_issuer_workload_public_key=$pem_public"
  "TC_SECRET_tc_staging_ingest_attestation_keypair=$attestation"
)

run_render() {
  # $1: output stem; the rest: NAME=value overrides.
  # With KEEP_OUT=1 the output directory is kept, as a rerun on the VM
  # finds it.
  local stem="$1"
  shift
  if [ "${KEEP_OUT:-0}" = 1 ]; then
    rm -rf "$site" "$deploy_dir"
  else
    rm -rf "$out" "$site" "$deploy_dir"
  fi
  mkdir -p "$out" "$site" "$deploy_dir"
  env -i PATH="/usr/bin:/bin:$(dirname "$envsubst_bin")" HOME="$scratch/home" \
    TC_RENDER_OUT_DIR="$out" TC_SITE_ROOT="$site" TC_DEPLOY_DIR="$deploy_dir" \
    TC_RENDER_SKIP_CHOWN=1 TC_LE_EMAIL=ops@example.invalid TC_GCP_REGION=us-central1 \
    "${secret_env[@]}" "$@" \
    bash ${RENDER_XTRACE:+-x} "$render" --secrets-from-env >"$scratch/$stem.out" 2>"$scratch/$stem.err"
}

if run_render render; then
  pass "render-env.sh renders with dummy secrets"
else
  fail "render-env.sh failed with dummy secrets"
  cat "$scratch/render.err" >&2
fi
ingest="$out/ingest.env"
ingest_default="$scratch/ingest-default.env"
[ -f "$ingest" ] && cp "$ingest" "$ingest_default"
issuer="$out/issuer.env"

value_of() {
  # The value of one assignment in a rendered env file, quotes as written.
  { grep -E "^$2=" "$1" || true; } | tail -n 1 | sed "s/^$2=//"
}

expect() {
  local file="$1" key="$2" want="$3" got
  got="$(value_of "$file" "$key")"
  if [ "$got" = "$want" ]; then
    pass "$(basename "$file") $key"
  else
    fail "$(basename "$file") $key is '$got', want '$want'"
  fi
}

if [ -f "$ingest" ]; then
  H=staging.tracecommons.ai
  expect "$ingest" TRACE_COMMONS_NEAR_AI_BASE_URL https://cloud-stg-api.near.ai/v1
  expect "$ingest" TRACE_COMMONS_NEAR_AI_MODEL Qwen/Qwen3.8-27B
  expect "$ingest" TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL https://cloud-stg-api.near.ai/v1
  expect "$ingest" TRACE_NEAR_AI_PRIVACY_BASE_URL https://cloud-stg-api.near.ai/v1
  expect "$ingest" TRACE_NEAR_AI_PRIVACY_MODEL openai/privacy-filter
  expect "$ingest" TRACE_PRIVACY_FILTER_BACKEND near-ai
  expect "$ingest" TRACE_COMMONS_NEAR_AI_API_KEY dummy-staging-near-ai-key
  expect "$ingest" TRACE_NEAR_AI_PRIVACY_API_KEY dummy-staging-near-ai-key
  expect "$ingest" TRACE_COMMONS_NEAR_PROVISIONING_ENABLED true
  expect "$ingest" TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL "https://issuer.$H"
  expect "$ingest" TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE trace-commons-ingest
  expect "$ingest" TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER ZHVtbXktc3RhZ2luZy1wZXBwZXItMDEyMzQ1Njc4OWFiY2RlZg==
  expect "$ingest" TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY "$hex_name_kek"
  expect "$ingest" TRACE_COMMONS_ADMISSION_ENABLED false
  expect "$ingest" TRACE_COMMONS_NEAR_AI_PAYOUT_MODE disabled
  expect "$ingest" TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED false
  expect "$ingest" TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS 0
  expect "$ingest" TRACE_COMMONS_GATE_POLICY_VERSION staging-2026-10
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_ID "$H"
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_ORIGIN "https://$H,https://ingest.$H"
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_NAME '"Near.ai Staging"'
  expect "$ingest" TRACE_COMMONS_SIGNED_TOKEN_ISSUER "https://issuer.$H"
  expect "$ingest" TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS "issuer.$H"
  expect "$ingest" DATABASE_URL postgres://app:dummystagingdbpassword@127.0.0.1:5432/tracecommons
  expect "$ingest" TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL postgres://tc_login_resolver_login:dummystagingresolverpassword@127.0.0.1:5432/tracecommons
  expect "$ingest" TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME projects/tracecommons-pilot-2026/locations/us-central1/keyRings/tc-staging/cryptoKeys/kek-v1
  expect "$ingest" TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET tc-staging-artifacts-20261021
  expect "$ingest" TRACE_COMMONS_REMOTE_OBJECT_STORE_CREDENTIAL_REF tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com
  expect "$ingest" TRACE_COMMONS_GATE_SERVICE_MASTER_KEY "$hex_gate"
  expect "$ingest" TRACE_COMMONS_ARTIFACT_KEY_HEX "$hex_artifact"
  expect "$ingest" TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID 00000000-0000-4000-8000-000000000002

  # Settings a staging ingest must not carry at all.
  for key in TRACE_COMMONS_COMMUNITY_TENANT_IDS TRACE_COMMONS_COMMUNITY_CORS_ORIGINS \
    TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON TRACE_COMMONS_NEAR_PROVISIONING_PUBLIC_ORIGIN; do
    if grep -q -E "^$key=" "$ingest"; then
      fail "the staging ingest env sets $key"
    else
      pass "the staging ingest env leaves $key unset"
    fi
  done
  if grep -q -E '^TRACE_COMMONS_WITNESS_' "$ingest"; then
    fail "the staging ingest env sets a TRACE_COMMONS_WITNESS_ setting"
  else
    pass "the staging ingest env sets no TRACE_COMMONS_WITNESS_ setting"
  fi

  # One assignment per key: an override replaced the pilot's line rather
  # than following it.
  dups="$(grep -E '^[A-Z_][A-Z0-9_]*=' "$ingest" | sed 's/=.*//' | sort | uniq -d)"
  if [ -z "$dups" ]; then
    pass "the staging ingest env assigns each key once"
  else
    fail "the staging ingest env assigns more than once: $dups"
  fi

  # Nothing of the pilot: its bucket, key ring, hosts, database or the
  # production NEAR AI hosts. The staging host is stripped first, so a bare
  # tracecommons.ai left over is a pilot host. The shared GCP project's name
  # (design 3.1: staging lives in the pilot's project) is stripped too, so
  # any other `pilot-` is a pilot resource or label. Comment lines are left out:
  # the Caddyfile keeps the pilot template's comments, which name examples.
  stripped="$(cat "$ingest" "$issuer" "$deploy_dir/Caddyfile" 2>/dev/null \
    | grep -v -E '^[[:space:]]*#' \
    | sed -e 's/staging\.tracecommons\.ai//g' -e 's/tracecommons-pilot-2026//g')"
  for needle in tc-pilot qwen3-6-35b cloud-api.near.ai completions.near.ai nip.io \
    tracecommons.ai '/trace-commons$' 'pilot-' '\$\{'; do
    if grep -q -E -- "$needle" <<<"$stripped"; then
      fail "a staging render names '$needle'"
      grep -n -E -- "$needle" <<<"$stripped" >&2 || true
    else
      pass "no staging render names '$needle'"
    fi
  done
fi

if [ -f "$issuer" ]; then
  expect "$issuer" TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER https://issuer.staging.tracecommons.ai
  expect "$issuer" TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID 00000000-0000-4000-8000-000000000001
  expect "$issuer" TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE trace-commons-ingest
  expect "$issuer" DATABASE_URL postgres://app:dummystagingdbpassword@127.0.0.1:5432/tracecommons
  expect "$issuer" TRACE_COMMONS_ONBOARDING_INGEST_URL https://ingest.staging.tracecommons.ai
  if grep -q -E '^TRACE_COMMONS_ALLOWLIST_SOURCE=' "$issuer"; then
    fail "the staging issuer env names an invite allowlist"
  else
    pass "the staging issuer env names no invite allowlist (open sign-up)"
  fi
fi

for pair in \
  "issuer-signing-v1.pem:-----BEGIN PRIVATE KEY-----" \
  "issuer-signing.pub:-----BEGIN PUBLIC KEY-----" \
  "workload-signing.pub:-----BEGIN PUBLIC KEY-----"; do
  name="${pair%%:*}"
  head_line="${pair#*:}"
  if [ -f "$out/$name" ] && [ "$(head -n 1 "$out/$name")" = "$head_line" ] \
    && [ "$(grep -c -- '-----' "$out/$name")" = 2 ]; then
    pass "render-env.sh wrote $name holding one PEM block"
  else
    fail "render-env.sh did not write $name as one PEM block"
  fi
done
for file in "$ingest" "$issuer" "$out/issuer-signing-v1.pem"; do
  [ -f "$file" ] || continue
  mode="$(stat -c '%a' "$file" 2>/dev/null || stat -f '%Lp' "$file")"
  if [ "$mode" = 640 ]; then
    pass "$(basename "$file") is mode 640"
  else
    fail "$(basename "$file") is mode $mode, want 640"
  fi
done
if grep -q -F -e dummy-staging -e dummystaging -e "$hex_gate" -e "$hex_artifact" \
  -e "$hex_name_kek" -e ZHVtbXkt -e 'PRIVATE KEY' "$scratch/render.out" "$scratch/render.err"; then
  fail "render-env.sh printed a secret value"
else
  pass "render-env.sh prints no secret value"
fi

# Run under `bash -x`, as an operator debugging it might, it still prints
# no secret: the script turns tracing off before it reads one.
if RENDER_XTRACE=1 run_render xtrace; then
  if grep -q -F -e dummy-staging -e dummystaging -e "$hex_gate" -e "$hex_artifact" \
    -e "$hex_name_kek" -e ZHVtbXkt -e 'PRIVATE KEY' "$scratch/xtrace.out" "$scratch/xtrace.err"; then
    fail "render-env.sh under bash -x printed a secret value"
  else
    pass "render-env.sh under bash -x prints no secret value"
  fi
else
  fail "render-env.sh failed under bash -x"
fi
run_render render >/dev/null || fail "render-env.sh failed on the second render"

aasa="$site/.well-known/apple-app-site-association"
if [ -f "$aasa" ] && [ "$(cat "$aasa")" = '{"webcredentials":{"apps":["KXSWJN7WY8.ai.tracecommons.shell.staging"]}}' ]; then
  pass "render-env.sh staged the staging AASA file"
else
  fail "render-env.sh did not stage the staging AASA file"
fi
if [ -d "$site/near-ai-pins" ]; then
  pass "render-env.sh made the signed pin directory"
else
  fail "render-env.sh did not make the signed pin directory"
fi

# Kept aside: the refusal runs below clear the deploy directory.
caddyfile="$scratch/staging-Caddyfile"
cp "$deploy_dir/Caddyfile" "$caddyfile" 2>/dev/null || fail "render-env.sh staged no Caddyfile"
if [ -f "$caddyfile" ]; then
  for site_name in "staging.tracecommons.ai {" "ingest.staging.tracecommons.ai {" "issuer.staging.tracecommons.ai {"; do
    if grep -q -x -F -- "$site_name" "$caddyfile"; then
      pass "the staging Caddyfile serves ${site_name% \{}"
    else
      fail "the staging Caddyfile has no site block for ${site_name% \{}"
    fi
  done
  for path in /.well-known/apple-app-site-association /near-ai-pins/current.json; do
    if grep -q -F -- "$path" "$caddyfile"; then
      pass "the staging Caddyfile routes $path"
    else
      fail "the staging Caddyfile does not route $path"
    fi
  done
  # The pilot's three blocks are the pilot template's, unchanged.
  if diff <(head -n "$(wc -l <"$repo/deploy/pilot-gcp/Caddyfile.template")" "$caddyfile" | sed 's/staging\.tracecommons\.ai/HOST/g') \
    <(env -i TC_PUBLIC_HOST=HOST TC_LE_EMAIL=ops@example.invalid "$envsubst_bin" <"$repo/deploy/pilot-gcp/Caddyfile.template") >/dev/null; then
    pass "the staging Caddyfile begins with the pilot template, unchanged"
  else
    fail "the staging Caddyfile does not begin with the pilot template"
  fi
fi
for unit in trace-commons-ingest.service trace-commons-upload-claim-issuer.service; do
  if cmp -s "$deploy_dir/$unit" "$repo/deploy/pilot-gcp/systemd/$unit"; then
    pass "render-env.sh staged the pilot's $unit for deploy.sh"
  else
    fail "render-env.sh did not stage the pilot's $unit"
  fi
done

# Expected measurements go in only when given, and a later render without
# them keeps them: rotating a secret must not quietly unpin the ingest.
pins="mrtd=$(printf '%096d' 0)"
if [ -n "$(value_of "$ingest" TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS)" ]; then
  fail "render-env.sh pinned measurements nobody gave it"
else
  pass "render-env.sh pins no measurements until given them"
fi
if run_render measured TC_NEAR_AI_EXPECTED_MEASUREMENTS="$pins"; then
  expect "$ingest" TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS "$pins"
else
  fail "render-env.sh failed with expected measurements"
fi
if KEEP_OUT=1 run_render rerun; then
  if [ "$(value_of "$ingest" TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS)" = "$pins" ]; then
    pass "a rerun without TC_NEAR_AI_EXPECTED_MEASUREMENTS keeps the pinned measurements"
  else
    fail "a rerun without TC_NEAR_AI_EXPECTED_MEASUREMENTS dropped the pinned measurements"
  fi
else
  fail "render-env.sh failed on a rerun after pinning measurements"
  cat "$scratch/rerun.err" >&2
fi
pins2="mrtd=$(printf '%096d' 1)"
if KEEP_OUT=1 run_render repin TC_NEAR_AI_EXPECTED_MEASUREMENTS="$pins2" \
  && KEEP_OUT=1 run_render rerun2; then
  expect "$ingest" TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS "$pins2"
else
  fail "render-env.sh failed to replace the pinned measurements"
fi
# A kept value that is not in the pin form refuses, as a given one does.
printf 'mrtd=not hex\n' >"$out/near-ai-expected-measurements"
if KEEP_OUT=1 run_render badpins; then
  fail "render-env.sh used kept measurements that are not in the pin form"
elif refused badpins; then
  pass "render-env.sh refuses kept measurements that are not in the pin form"
else
  fail "render-env.sh failed on malformed kept measurements without refusing"
fi

# A missing or empty secret refuses and leaves nothing behind.
for missing in TC_SECRET_tc_staging_gate_master_key TC_SECRET_tc_staging_near_account_index_pepper; do
  if run_render missing "$missing="; then
    fail "render-env.sh rendered with $missing empty"
  elif ! refused missing; then
    fail "render-env.sh failed on an empty $missing without refusing"
  elif [ -e "$out/ingest.env" ] || [ -e "$out/issuer.env" ]; then
    fail "render-env.sh left an env file behind after refusing on $missing"
  else
    pass "render-env.sh refuses an empty ${missing#TC_SECRET_} and writes nothing"
  fi
done
# A malformed issuer keypair refuses.
if run_render badkey TC_SECRET_tc_staging_issuer_signing_keypair="kid: x"; then
  fail "render-env.sh accepted an issuer keypair with no PEM"
elif ! refused badkey; then
  fail "render-env.sh failed on an issuer keypair with no PEM without refusing"
else
  pass "render-env.sh refuses an issuer keypair with no PEM"
fi
# The attestation secret carries exactly its three settings.
if run_render badattest "TC_SECRET_tc_staging_ingest_attestation_keypair=$attestation"$'\nTRACE_COMMONS_NEAR_AI_BASE_URL=https://cloud-api.near.ai/v1'; then
  fail "render-env.sh accepted an attestation secret carrying another setting"
elif ! refused badattest; then
  fail "render-env.sh failed on a widened attestation secret without refusing"
else
  pass "render-env.sh refuses an attestation secret carrying another setting"
fi
# Payout in dry_run needs its settings and the reseller key.
if run_render payout TC_NEAR_AI_PAYOUT_MODE=dry_run; then
  fail "render-env.sh enabled payout without its settings"
elif ! refused payout; then
  fail "render-env.sh failed on payout dry_run without refusing"
else
  pass "render-env.sh refuses payout dry_run without its settings"
fi
# With every setting present, the reseller key must have the shape cloud-api
# issues an org credential in: `oc_` and 64 hex. An API key (`sk-...`) or any
# other token is refused before anything is written, and the source must be
# one #1271 names. Synthetic values only.
# The settlement table carries the payout rate; the render checks only that
# the file is there, the ingest parses it at boot. A placeholder document.
payout_table="$scratch/settlement-table.json"
echo '{}' >"$payout_table"
payout_settings=(
  TC_NEAR_AI_PAYOUT_MODE=dry_run
  TC_NEAR_AI_PAYOUT_CREDENTIAL_ID=00000000-0000-4000-8000-0000000000c1
  TC_NEAR_AI_PAYOUT_SENDER_ORG=00000000-0000-4000-8000-0000000000c2
  TC_NEAR_AI_PAYOUT_SOURCE=payment
  TC_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO=1000
  "TC_CREDIT_SETTLEMENT_TABLE_PATH=$payout_table"
)
payout_key_ok="oc_$(printf 'c3%.0s' $(seq 32))"
if run_render payout-ok "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_MODE dry_run
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL "$payout_key_ok"
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE payment
  expect "$out/ingest.env" TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH "$payout_table"
  # The recipient key the link routes seal with: sign-up provisioning, the
  # pepper and the account-name KEK all reach an enabled rail's ingest.
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_PROVISIONING_ENABLED true
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER ZHVtbXktc3RhZ2luZy1wZXBwZXItMDEyMzQ1Njc4OWFiY2RlZg==
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY "$hex_name_kek"
else
  fail "render-env.sh failed on payout dry_run with every setting"
  cat "$scratch/payout-ok.err" >&2
fi
for source in grant staking_farm; do
  if run_render "payout-$source" "${payout_settings[@]}" \
    "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
    "TC_NEAR_AI_PAYOUT_SOURCE=$source"; then
    expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE "$source"
  else
    fail "render-env.sh refused payout source $source"
  fi
done
if run_render payout-upper "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=oc_$(printf 'C3%.0s' $(seq 32))"; then
  pass "render-env.sh takes an oc_ credential in upper-case hex"
else
  fail "render-env.sh refused an oc_ credential in upper-case hex"
fi
payout_refused() {
  # $1: case name; the rest: NAME=value assignments.
  local name="$1"
  shift
  if run_render "payout-$name" "${payout_settings[@]}" "$@"; then
    fail "render-env.sh accepted payout settings ($name)"
  elif ! refused "payout-$name"; then
    fail "render-env.sh failed on payout settings ($name) without refusing"
  else
    pass "render-env.sh refuses payout settings ($name)"
  fi
}
payout_refused api-key "TC_SECRET_tc_staging_nearai_reseller_key=sk-$(printf 'c3%.0s' $(seq 32))"
payout_refused short-hex "TC_SECRET_tc_staging_nearai_reseller_key=oc_$(printf 'c3%.0s' $(seq 31))"
payout_refused long-hex "TC_SECRET_tc_staging_nearai_reseller_key=${payout_key_ok}c3"
payout_refused not-hex "TC_SECRET_tc_staging_nearai_reseller_key=oc_$(printf 'g3%.0s' $(seq 32))"
payout_refused prefix "TC_SECRET_tc_staging_nearai_reseller_key=OC_$(printf 'c3%.0s' $(seq 32))"
payout_refused source "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_PAYOUT_SOURCE=transfer
payout_refused no-source "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_PAYOUT_SOURCE=
# An enabled rail without a settlement table records no rate and pays
# nothing, so the render refuses it: unset, relative, or a file that is not
# there.
payout_refused no-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_CREDIT_SETTLEMENT_TABLE_PATH=
payout_refused relative-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_CREDIT_SETTLEMENT_TABLE_PATH=settlement-table.json
payout_refused missing-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_CREDIT_SETTLEMENT_TABLE_PATH=$scratch/no-such-table.json"
# Every setting the rail reads from the environment is one the render knows
# (NearAiRailConfig's names, the rate table's path), so a setting added later
# cannot reach an enabled rail unrendered.
rail_config="$repo/crates/trace-commons-server/src/near_ai_rail_config.rs"
rail_names="$(grep -E '^pub const NEAR_AI_PAYOUT_[A-Z_]+_ENV: &str' -A1 "$rail_config" \
  | grep -oE '"TRACE_COMMONS_[A-Z0-9_]+"' | tr -d '"')"
rail_names+=$'\n'"$(grep -E '^pub const NEAR_AI_PAYOUT_MODE_ENV' "$repo/crates/trace-commons-server/src/payout_rate.rs" \
  | grep -oE '"TRACE_COMMONS_[A-Z0-9_]+"' | tr -d '"')"
rail_names+=$'\nTRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH'
if [ "$(printf '%s\n' "$rail_names" | grep -c .)" -lt 7 ]; then
  fail "found fewer than seven payout settings in the rail config ($(printf '%s\n' "$rail_names" | grep -c .))"
fi
while IFS= read -r name; do
  [ -n "$name" ] || continue
  if grep -q "$name" "$repo/deploy/staging-gcp/render-env.sh" \
    && grep -q "$name" "$repo/deploy/staging-gcp/ingest.env.template"; then
    pass "the staging render and template name $name"
  else
    fail "the staging render or template does not name $name"
  fi
done <<<"$rail_names"

# The first run's dollar estimate: unset by default, so the ingest serves no
# range and the app words its first run in points. Both settings pass
# through as given; one without the other, or anything but digits, refuses.
# Synthetic values: not a rate anyone runs with.
first_run_keys='^TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_(MIN|MAX)='
if grep -Eq "$first_run_keys" "$ingest_default"; then
  fail "the default staging render sets the first-run dollar estimate"
else
  pass "the default staging render leaves the first-run dollar estimate unset"
fi
if render_pilot "$repo/deploy/pilot-gcp/ingest.env.template" | grep -Eq "$first_run_keys"; then
  fail "the pilot render sets the first-run dollar estimate"
else
  pass "the pilot render leaves the first-run dollar estimate unset"
fi
if run_render firstrun TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN=120000 TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX=340000; then
  expect "$out/ingest.env" TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN 120000
  expect "$out/ingest.env" TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MAX 340000
else
  fail "render-env.sh failed with a first-run dollar estimate"
  cat "$scratch/firstrun.err" >&2
fi
first_run_refused() {
  # $1: case name; the rest: NAME=value assignments.
  local name="$1"
  shift
  if run_render "firstrun-$name" "$@"; then
    fail "render-env.sh accepted a first-run dollar estimate ($name)"
  elif ! refused "firstrun-$name"; then
    fail "render-env.sh failed on a first-run dollar estimate ($name) without refusing"
  else
    pass "render-env.sh refuses a first-run dollar estimate ($name)"
  fi
}
first_run_refused min-only TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN=120000
first_run_refused max-only TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX=340000
first_run_refused not-digits TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN=0.12 TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX=340000
first_run_refused newline TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN=120000$'\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' \
  TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX=340000

# Secrets that go into a KEY=value line or a URL have their shape checked
# by name: a password that would break the database URL, a key that is not
# hex, and any single-line secret carrying a newline, which would add a line
# of its own to the env file (here, a witness bypass).
for bad in \
  "db_app_password:TC_SECRET_tc_staging_db_app_password=pass@word/x" \
  "db_login_resolver_password:TC_SECRET_tc_staging_db_login_resolver_password=pass:word" \
  "gate_master_key:TC_SECRET_tc_staging_gate_master_key=$(printf 'zz%.0s' $(seq 32))" \
  "artifact_key:TC_SECRET_tc_staging_artifact_key=${hex_artifact:0:62}" \
  "near_account_name_kek_key:TC_SECRET_tc_staging_near_account_name_kek_key=$hex_name_kek"$'\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' \
  "nearai_inference_key:TC_SECRET_tc_staging_nearai_inference_key=dummy-staging-near-ai-key"$'\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' \
  "near_account_index_pepper:TC_SECRET_tc_staging_near_account_index_pepper=ZHVtbXktc3RhZ2luZy1wZXBwZXItMDEyMzQ1Njc4OWFiY2RlZg=="$'\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true'; do
  name="${bad%%:*}"
  if run_render "bad-$name" "${bad#*:}"; then
    fail "render-env.sh accepted a malformed tc-staging-${name//_/-}"
  elif ! grep -q "^render-env: refusing: secret tc-staging-${name//_/-} " "$scratch/bad-$name.err"; then
    fail "render-env.sh did not refuse a malformed tc-staging-${name//_/-} by name"
    cat "$scratch/bad-$name.err" >&2
  elif [ -e "$out/ingest.env" ] || [ -e "$out/issuer.env" ]; then
    fail "render-env.sh left an env file behind after refusing tc-staging-${name//_/-}"
  else
    pass "render-env.sh refuses a malformed tc-staging-${name//_/-} by name and writes nothing"
  fi
done

# Read through gcloud, a refusal says why a secret could not be read: a
# permission denial is not a missing secret. gcloud's message names the
# secret's full resource path; only its error class is repeated.
gstubs="$scratch/gcloud-stubs"
mkdir -p "$gstubs"
cat >"$gstubs/gcloud" <<'STUB'
#!/usr/bin/env bash
echo "ERROR: (gcloud.secrets.versions.access) $FAKE_GCLOUD_ERROR: Permission 'secretmanager.versions.access' denied for resource 'projects/000000000000/secrets/x/versions/latest'" >&2
exit 1
STUB
chmod +x "$gstubs/gcloud"
for class in PERMISSION_DENIED NOT_FOUND; do
  rm -rf "$out" "$site" "$deploy_dir"
  mkdir -p "$out" "$site" "$deploy_dir"
  if env -i PATH="$gstubs:/usr/bin:/bin:$(dirname "$envsubst_bin")" HOME="$scratch/home" \
    TC_RENDER_OUT_DIR="$out" TC_SITE_ROOT="$site" TC_DEPLOY_DIR="$deploy_dir" \
    TC_RENDER_SKIP_CHOWN=1 TC_LE_EMAIL=ops@example.invalid TC_GCP_REGION=us-central1 \
    FAKE_GCLOUD_ERROR="$class" \
    bash "$render" >"$scratch/gcloud.out" 2>"$scratch/gcloud.err"; then
    fail "render-env.sh rendered with gcloud failing $class"
  elif grep -qx "render-env: refusing: secret tc-staging-db-app-password could not be read ($class)" "$scratch/gcloud.err" \
    && ! grep -q 'projects/' "$scratch/gcloud.err"; then
    pass "render-env.sh names gcloud's $class, and nothing else of its message"
  else
    fail "render-env.sh did not name gcloud's $class alone"
    cat "$scratch/gcloud.err" >&2
  fi
done

# 4. caddy adapt, where caddy is installed.
if command -v caddy >/dev/null 2>&1; then
  for file in "$scratch/pilot-Caddyfile" "$caddyfile"; do
    if [ ! -f "$file" ]; then
      fail "no $(basename "$file") to adapt"
      continue
    fi
    if caddy adapt --config "$file" --adapter caddyfile >/dev/null 2>"$scratch/adapt.err"; then
      pass "caddy adapt accepts the $(basename "$file")"
    else
      fail "caddy adapt refuses $file"
      cat "$scratch/adapt.err" >&2
    fi
  done
else
  echo "SKIP: caddy is not installed; caddy adapt not run"
fi

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all deploy render checks passed"
