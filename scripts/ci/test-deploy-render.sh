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
    */health)
      # FAKE_HEALTH_REFUSALS: the ingest (port 3907) refuses that many
      # connections first, as one still applying migrations does after its
      # restart. Other services answer at once.
      if [ -n "${FAKE_HEALTH_REFUSALS:-}" ] && [[ "$arg" == *:3907/health ]]; then
        n="$(cat "$TRACE.refused" 2>/dev/null || echo 0)"
        if [ "$n" -lt "$FAKE_HEALTH_REFUSALS" ]; then
          echo $((n + 1)) >"$TRACE.refused"
          echo "curl: (7) Failed to connect" >&2
          exit 7
        fi
      fi
      printf '%s' "$FAKE_HEALTH"; exit 0 ;;
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

# A first install applies every migration, so the ingest may refuse
# connections for a while after its start: deploy.sh asks again rather than
# ending on the first refusal, and bounds each ask.
rm -f "$scratch/slow.trace.refused"
if run_deploy "$scratch/slow.trace" "$pilot_health" FAKE_HEALTH_REFUSALS=5 \
  && grep -q 'DEPLOY OK' "$scratch/slow.trace.out"; then
  pass "pilot deploy.sh waits for a slow ingest's /health"
else
  fail "pilot deploy.sh ended on an ingest that was still starting"
  cat "$scratch/slow.trace.err" >&2
fi
if grep -q -- '--connect-timeout 2 --max-time 5 http://127.0.0.1:3907/health' "$scratch/slow.trace"; then
  pass "pilot deploy.sh bounds each /health ask"
else
  fail "pilot deploy.sh asks /health with no time limit"
fi
# One that never answers is refused and taken down, never left running.
rm -f "$scratch/silent.trace.refused"
if run_deploy "$scratch/silent.trace" "$pilot_health" FAKE_HEALTH_REFUSALS=1000; then
  fail "pilot deploy.sh carried on with an ingest whose /health never answered"
elif grep -q 'did not answer' "$scratch/silent.trace.err" \
  && grep -qx 'sudo systemctl stop trace-commons-ingest.service' "$scratch/silent.trace"; then
  pass "pilot deploy.sh refuses and stops an ingest whose /health never answers"
else
  fail "pilot deploy.sh failed for some other reason when /health never answered"
  cat "$scratch/silent.trace.err" >&2
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
# A first boot that applies migrations is not listening yet when the
# check starts (2026-10-27: V139 and V140 on staging, refused once, and the
# deploy rolled a good build back). The check waits for /health to answer.
rm -f "$scratch/pull-slow.trace.refused"
if run_pull "$scratch/pull-slow.trace" "$staging_health" TC_EXPECT_STAGING=1 FAKE_HEALTH_REFUSALS=5; then
  if ! grep -qxF "$rollback" "$scratch/pull-slow.trace"; then
    pass "pull-and-install.sh with TC_EXPECT_STAGING=1 waits for a slow ingest's /health"
    if grep -q -- '--connect-timeout 2 --max-time 5 http://127.0.0.1:3907/health' "$scratch/pull-slow.trace"; then
      pass "pull-and-install.sh bounds each /health ask"
    else
      fail "pull-and-install.sh asks /health with no time limit"
    fi
  else
    fail "pull-and-install.sh rolled back an ingest that was still starting"
  fi
else
  fail "pull-and-install.sh with TC_EXPECT_STAGING=1 refused an ingest that was still starting"
  cat "$scratch/pull-slow.trace.err" >&2
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
  expect "$ingest" TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS 100000
  expect "$ingest" TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED false
  expect "$ingest" TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS 0
  expect "$ingest" TRACE_COMMONS_GATE_POLICY_VERSION staging-2026-10
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
  expect "$issuer" TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB true
  # One sign-up policy template, the same in both processes.
  if [ -n "$(value_of "$issuer" TRACE_COMMONS_NEAR_AI_SIGNUP_POLICY_TEMPLATE)" ] && [ -f "$ingest" ] \
    && [ "$(value_of "$issuer" TRACE_COMMONS_NEAR_AI_SIGNUP_POLICY_TEMPLATE)" = "$(value_of "$ingest" TRACE_COMMONS_NEAR_AI_SIGNUP_POLICY_TEMPLATE)" ]; then
    pass "the staging ingest and issuer carry one NEAR AI sign-up policy template"
  else
    fail "the staging ingest and issuer NEAR AI sign-up policy templates are missing or differ"
  fi
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
  # dry_run sends nothing, so it renders without the operator's control
  # token and does not read its secret.
  if grep -q '^TRACE_COMMONS_PAYOUT_CONTROL_TOKEN=' "$out/ingest.env"; then
    fail "render-env.sh wrote a payout control token for dry_run"
  else
    pass "render-env.sh renders payout dry_run without the control token"
  fi
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
# An enabled rail without a settlement table has no rate in force: each pass
# stops with payout_rate_unavailable, even over a rate epoch an earlier boot
# stored. The render refuses it: unset, relative, or a file that is not
# there.
payout_refused no-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_CREDIT_SETTLEMENT_TABLE_PATH=
payout_refused relative-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_CREDIT_SETTLEMENT_TABLE_PATH=settlement-table.json
payout_refused missing-table "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_CREDIT_SETTLEMENT_TABLE_PATH=$scratch/no-such-table.json"
# A live rail needs the operator's payout control token, or its kill switch
# is unreachable (the ingest refuses to boot without it): the render reads
# tc-staging-payout-control-token, 32 or more characters on one line, and
# refuses without it. Synthetic tokens only.
control_token_ok="$(printf 'k7%.0s' $(seq 20))"
if run_render payout-live "${payout_settings[@]}" TC_NEAR_AI_PAYOUT_MODE=live \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_SECRET_tc_staging_payout_control_token=$control_token_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_MODE live
  expect "$out/ingest.env" TRACE_COMMONS_PAYOUT_CONTROL_TOKEN "$control_token_ok"
else
  fail "render-env.sh failed on payout live with every setting"
  cat "$scratch/payout-live.err" >&2
fi
payout_refused live-no-control-token TC_NEAR_AI_PAYOUT_MODE=live \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok"
if grep -q 'tc-staging-payout-control-token' "$scratch/payout-live-no-control-token.err"; then
  pass "the live refusal names the control token secret"
else
  fail "the live refusal does not name the control token secret"
fi
payout_refused live-short-control-token TC_NEAR_AI_PAYOUT_MODE=live \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_SECRET_tc_staging_payout_control_token=$(printf 'k%.0s' $(seq 31))"
payout_refused live-two-line-control-token TC_NEAR_AI_PAYOUT_MODE=live \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_SECRET_tc_staging_payout_control_token=$control_token_ok"$'\n'"TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=disabled"
payout_refused live-spaced-control-token TC_NEAR_AI_PAYOUT_MODE=live \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  "TC_SECRET_tc_staging_payout_control_token=$control_token_ok with spaces"
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

# Referral rewards (referral spec sections 3-7). Off by default: no program
# start, so no code can be made or applied, and the bonus stream disabled.
# The ingest reads unset as off and a present-but-malformed value as a
# refusal to boot, so the render writes a setting only when it is given.
referral_program_keys='^TRACE_COMMONS_REFERRAL_(PROGRAM_START|PROGRAM_END|LATE_DAYS|EARN_DAYS|BONUS_BPS|NETWORK_MARK_KEY|ACCOUNT_CEILING_NANO)='
expect "$ingest_default" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE disabled
if grep -Eq "$referral_program_keys|^TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL" "$ingest_default"; then
  fail "the default staging render sets a referral setting other than the disabled mode"
else
  pass "the default staging render leaves referrals off"
fi
if render_pilot "$repo/deploy/pilot-gcp/ingest.env.template" \
  | grep -Eq "$referral_program_keys|^TRACE_COMMONS_NEAR_AI_REFERRAL_(MODE|CREDENTIAL)"; then
  fail "the pilot render turns a referral setting on"
else
  pass "the pilot render leaves referrals off"
fi
# Synthetic values only. The mark key is base64 of 33 bytes; the referral
# credential is an org credential distinct from the reseller key.
mark_key_ok="$(printf 'bW1t%.0s' $(seq 11))"
referral_key_ok="oc_$(printf 'e5%.0s' $(seq 32))"
referral_refused() {
  # $1: case name; the rest: NAME=value assignments.
  local name="$1"
  shift
  if run_render "referral-$name" "$@"; then
    fail "render-env.sh accepted referral settings ($name)"
  elif ! refused "referral-$name"; then
    fail "render-env.sh failed on referral settings ($name) without refusing"
    cat "$scratch/referral-$name.err" >&2
  else
    pass "render-env.sh refuses referral settings ($name)"
  fi
}
no_secret_printed() {
  # $1: output stem; the rest: secret values that must not appear.
  local stem="$1" value
  shift
  for value in "$@"; do
    if grep -q -F "$value" "$scratch/$stem.out" "$scratch/$stem.err"; then
      fail "render-env.sh printed a referral secret ($stem)"
      return
    fi
  done
  pass "render-env.sh prints no referral secret ($stem)"
}

# The program alone (codes, no bonus stream): its start and the network
# mark key, which the ingest requires whenever referrals are configured.
program_settings=(
  TC_REFERRAL_PROGRAM_START=2026-11-01T00:00:00Z
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"
)
if run_render referral-program "${program_settings[@]}" \
  TC_REFERRAL_PROGRAM_END=2027-11-01T00:00:00Z TC_REFERRAL_LATE_DAYS=30 \
  TC_REFERRAL_EARN_DAYS=120 TC_REFERRAL_BONUS_BPS=1000; then
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_PROGRAM_START 2026-11-01T00:00:00Z
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_PROGRAM_END 2027-11-01T00:00:00Z
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_LATE_DAYS 30
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_EARN_DAYS 120
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_BONUS_BPS 1000
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY "$mark_key_ok"
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE disabled
  no_secret_printed referral-program "$mark_key_ok"
else
  fail "render-env.sh failed on the referral program with its settings"
  cat "$scratch/referral-program.err" >&2
fi
if run_render referral-program-only-start "${program_settings[@]}"; then
  if grep -Eq '^TRACE_COMMONS_REFERRAL_(PROGRAM_END|LATE_DAYS|EARN_DAYS|BONUS_BPS)=' "$out/ingest.env"; then
    fail "render-env.sh wrote an optional referral setting nobody gave"
  else
    pass "render-env.sh leaves the optional referral settings to the ingest's defaults"
  fi
else
  fail "render-env.sh failed on the referral program with only its start"
fi
referral_refused program-no-mark-key TC_REFERRAL_PROGRAM_START=2026-11-01T00:00:00Z
if grep -q 'tc-staging-referral-network-mark-key' "$scratch/referral-program-no-mark-key.err"; then
  pass "the referral refusal names the network mark key secret"
else
  fail "the referral refusal does not name the network mark key secret"
fi
referral_refused program-bad-start "${program_settings[@]}" TC_REFERRAL_PROGRAM_START=tomorrow
referral_refused program-bad-earn-days "${program_settings[@]}" TC_REFERRAL_EARN_DAYS=92d
referral_refused program-bad-mark-key TC_REFERRAL_PROGRAM_START=2026-11-01T00:00:00Z \
  "TC_SECRET_tc_staging_referral_network_mark_key=short"
referral_refused program-two-line-mark-key TC_REFERRAL_PROGRAM_START=2026-11-01T00:00:00Z \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"$'\n'"TRACE_COMMONS_NEAR_AI_REFERRAL_MODE=live"

# The bonus stream. It rides the NEAR AI rail, so it needs the rail on, and
# live needs the rail live; the ingest refuses either at boot.
referral_refused stream-without-rail TC_NEAR_AI_REFERRAL_MODE=dry_run \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"
referral_refused stream-bad-mode "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_REFERRAL_MODE=on \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"
referral_refused stream-no-mark-key "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_REFERRAL_MODE=dry_run
if run_render referral-dry-run "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_REFERRAL_MODE=dry_run TC_REFERRAL_ACCOUNT_CEILING_NANO=5000 \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE dry_run
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO 5000
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY "$mark_key_ok"
  # A dry run sends nothing, so it neither needs nor reads the referral
  # credential.
  if grep -q '^TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL' "$out/ingest.env"; then
    fail "render-env.sh wrote a referral credential for dry_run"
  else
    pass "render-env.sh renders the referral stream's dry_run without its credential"
  fi
else
  fail "render-env.sh failed on the referral stream's dry_run with every setting"
  cat "$scratch/referral-dry-run.err" >&2
fi
referral_live=(
  "${payout_settings[@]}" TC_NEAR_AI_PAYOUT_MODE=live
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok"
  "TC_SECRET_tc_staging_payout_control_token=$control_token_ok"
  TC_NEAR_AI_REFERRAL_MODE=live
  TC_NEAR_AI_REFERRAL_CREDENTIAL_ID=00000000-0000-4000-8000-0000000000c3
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"
)
if run_render referral-live "${referral_live[@]}" \
  "TC_SECRET_tc_staging_nearai_referral_key=$referral_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE live
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL "$referral_key_ok"
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL_ID 00000000-0000-4000-8000-0000000000c3
  no_secret_printed referral-live "$referral_key_ok" "$mark_key_ok"
else
  fail "render-env.sh failed on the referral stream live with every setting"
  cat "$scratch/referral-live.err" >&2
fi
# Staging as configured on 2026-10-29: the own-trace rail live, the program
# started, the bonus stream in dry_run, and no referral credential.
mark_key_padded="$(printf 'bW1t%.0s' $(seq 10))bW0="
if run_render referral-staging-2026-10-29 "${referral_live[@]}" \
  TC_NEAR_AI_REFERRAL_MODE=dry_run TC_NEAR_AI_REFERRAL_CREDENTIAL_ID= \
  TC_REFERRAL_PROGRAM_START=2026-10-01T00:00:00Z \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_padded"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_PAYOUT_MODE live
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE dry_run
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_PROGRAM_START 2026-10-01T00:00:00Z
  expect "$out/ingest.env" TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY "$mark_key_padded"
  if grep -q '^TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL' "$out/ingest.env"; then
    fail "render-env.sh wrote a referral credential for staging's dry_run"
  else
    pass "render-env.sh renders staging's referral configuration without a credential"
  fi
else
  fail "render-env.sh refused staging's referral configuration"
  cat "$scratch/referral-staging-2026-10-29.err" >&2
fi
referral_refused live-on-dry-run-rail "${payout_settings[@]}" \
  "TC_SECRET_tc_staging_nearai_reseller_key=$payout_key_ok" \
  TC_NEAR_AI_REFERRAL_MODE=live \
  TC_NEAR_AI_REFERRAL_CREDENTIAL_ID=00000000-0000-4000-8000-0000000000c3 \
  "TC_SECRET_tc_staging_nearai_referral_key=$referral_key_ok" \
  "TC_SECRET_tc_staging_referral_network_mark_key=$mark_key_ok"
referral_refused live-no-key "${referral_live[@]}"
if grep -q 'tc-staging-nearai-referral-key' "$scratch/referral-live-no-key.err"; then
  pass "the referral live refusal names the referral credential secret"
else
  fail "the referral live refusal does not name the referral credential secret"
fi
# Live with no referral credential sends with the payout credential (owner
# decision, 2026-10-29): nothing more is rendered and no referral secret is
# read, even one that is present.
if run_render referral-live-shared "${referral_live[@]}" TC_NEAR_AI_REFERRAL_CREDENTIAL_ID= \
  "TC_SECRET_tc_staging_nearai_referral_key=$referral_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_MODE live
  if grep -q '^TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL' "$out/ingest.env"; then
    fail "render-env.sh wrote a referral credential for a live stream on the payout credential"
  else
    pass "render-env.sh renders a live referral stream on the payout credential"
  fi
  no_secret_printed referral-live-shared "$referral_key_ok" "$mark_key_ok"
else
  fail "render-env.sh refused a live referral stream on the payout credential"
  cat "$scratch/referral-live-shared.err" >&2
fi
# Naming the payout pair explicitly is the same choice, and renders.
if run_render referral-live-payout-pair "${referral_live[@]}" \
  TC_NEAR_AI_REFERRAL_CREDENTIAL_ID=00000000-0000-4000-8000-0000000000c1 \
  "TC_SECRET_tc_staging_nearai_referral_key=$payout_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL "$payout_key_ok"
else
  fail "render-env.sh refused the payout pair named as the referral credential"
  cat "$scratch/referral-live-payout-pair.err" >&2
fi
referral_refused live-api-key "${referral_live[@]}" \
  "TC_SECRET_tc_staging_nearai_referral_key=sk-$(printf 'e5%.0s' $(seq 32))"
# Half of the payout pair beside half of another is a mistake: refused.
referral_refused live-reseller-key "${referral_live[@]}" \
  "TC_SECRET_tc_staging_nearai_referral_key=$payout_key_ok"
referral_refused live-same-credential-id "${referral_live[@]}" \
  "TC_SECRET_tc_staging_nearai_referral_key=$referral_key_ok" \
  TC_NEAR_AI_REFERRAL_CREDENTIAL_ID=00000000-0000-4000-8000-0000000000c1

# Every referral setting the ingest reads is one the render and the staging
# template name, so a setting added later cannot reach a configured program
# unrendered. The abuse thresholds are tuning with defaults in code and are
# documented in docs/operator/env-reference.md, not rendered.
referral_names="$(
  for f in referral_config.rs near_ai_rail_config.rs referral_network.rs; do
    grep -E '^pub const [A-Z_]+_ENV: &str' -A1 "$repo/crates/trace-commons-server/src/$f"
  done | grep -oE '"TRACE_COMMONS_(NEAR_AI_)?REFERRAL_[A-Z0-9_]+"' | tr -d '"' | sort -u
)"
if [ "$(printf '%s\n' "$referral_names" | grep -c .)" -lt 10 ]; then
  fail "found fewer than ten referral settings in the referral config ($(printf '%s\n' "$referral_names" | grep -c .))"
fi
while IFS= read -r name; do
  [ -n "$name" ] || continue
  if grep -q "$name" "$repo/deploy/staging-gcp/render-env.sh" \
    && grep -q "$name" "$repo/deploy/staging-gcp/ingest.env.template" \
    && grep -q "$name" "$repo/docs/operator/env-reference.md"; then
    pass "the staging render, template and env reference name $name"
  else
    fail "the staging render, template or env reference does not name $name"
  fi
done <<<"$referral_names"

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

# The scoring driver and the scoring award (the staging settlement test).
# Off by default: a default render names none of the four settings. Synthetic
# values only.
scoring_keys='^TRACE_COMMONS_(PERPLEXITY_DRIVER_ENABLED|GATE_DRIVER_DATABASE_URL|NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE|NOVELTY_UTILITY_SETTLE_FROM)='
if grep -Eq "$scoring_keys" "$ingest_default"; then
  fail "the default staging render sets a scoring driver or scoring award setting"
else
  pass "the default staging render leaves the scoring driver and award settings unset"
fi
expect "$ingest_default" TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA 0

gate_driver_ok=dummystaginggatedriverpassword
scoring_refused() {
  # $1: stem; the rest: render overrides. A refusal must name itself.
  local stem="$1"
  shift
  if run_render "$stem" "$@"; then
    fail "render-env.sh accepted $stem"
  elif ! refused "$stem"; then
    fail "render-env.sh failed on $stem without refusing"
  else
    pass "render-env.sh refuses $stem"
  fi
}
no_duplicate_keys() {
  local dups
  dups="$(grep -E '^[A-Z_][A-Z0-9_]*=' "$1" | sed 's/=.*//' | sort | uniq -d)"
  if [ -z "$dups" ]; then
    pass "$2 assigns no key twice"
  else
    fail "$2 assigns a key twice: $dups"
  fi
}

if run_render driver-on TC_SCORING_DRIVER=1 \
  "TC_SECRET_tc_staging_db_gate_driver_password=$gate_driver_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED true
  expect "$out/ingest.env" TRACE_COMMONS_GATE_DRIVER_DATABASE_URL \
    "postgres://tc_gate_driver_login:$gate_driver_ok@127.0.0.1:5432/tracecommons"
  expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA 0
  no_duplicate_keys "$out/ingest.env" "the scoring driver render"
else
  fail "render-env.sh failed with the scoring driver on"
  cat "$scratch/driver-on.err" >&2
fi
scoring_refused driver-no-secret TC_SCORING_DRIVER=1
if grep -q 'tc-staging-db-gate-driver-password' "$scratch/driver-no-secret.err"; then
  pass "the scoring driver refusal names the gate driver secret"
else
  fail "the scoring driver refusal does not name the gate driver secret"
fi
scoring_refused driver-bad-password TC_SCORING_DRIVER=1 \
  "TC_SECRET_tc_staging_db_gate_driver_password=pass/word@host:1"
scoring_refused driver-bad-toggle TC_SCORING_DRIVER=yes \
  "TC_SECRET_tc_staging_db_gate_driver_password=$gate_driver_ok"
# Without the toggle the secret is not read, set or not.
if run_render driver-off-secret-set \
  "TC_SECRET_tc_staging_db_gate_driver_password=$gate_driver_ok" \
  && ! grep -Eq "$scoring_keys" "$out/ingest.env"; then
  pass "a render without TC_SCORING_DRIVER ignores the gate driver secret"
else
  fail "a render without TC_SCORING_DRIVER wrote scoring settings"
fi

settle_ok=2026-11-01T00:00:00Z
scoring_refused delta-no-settle-from TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=5
if grep -q 'TC_NOVELTY_UTILITY_SETTLE_FROM' "$scratch/delta-no-settle-from.err"; then
  pass "the delta refusal names TC_NOVELTY_UTILITY_SETTLE_FROM"
else
  fail "the delta refusal does not name TC_NOVELTY_UTILITY_SETTLE_FROM"
fi
for bad in 2026-11-01 "2026-11-01 00:00:00Z" 2026-11-01T00:00:00 2026-11-01T00:00:00+02:00 \
  2026-13-01T00:00:00Z 2026-11-32T00:00:00Z 2026-11-01T24:00:00Z tomorrow; do
  scoring_refused "settle-from-invalid-${bad//[^0-9A-Za-z]/_}" \
    TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=5 "TC_NOVELTY_UTILITY_SETTLE_FROM=$bad"
done
for bad in -1 abc 1e3 "5 6"; do
  scoring_refused "delta-invalid-${bad//[^0-9A-Za-z]/_}" \
    "TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=$bad" "TC_NOVELTY_UTILITY_SETTLE_FROM=$settle_ok"
done
for good in 5 0.5 12.25; do
  if run_render "delta-$good" "TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=$good" \
    "TC_NOVELTY_UTILITY_SETTLE_FROM=$settle_ok"; then
    expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA "$good"
    expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE true
    expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM "$settle_ok"
    no_duplicate_keys "$out/ingest.env" "the delta $good render"
  else
    fail "render-env.sh failed with delta $good and a cut-over"
    cat "$scratch/delta-$good.err" >&2
  fi
done
# A zero delta needs neither control and renders none.
if run_render delta-zero TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0 \
  && ! grep -Eq "$scoring_keys" "$out/ingest.env"; then
  expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA 0
else
  fail "render-env.sh failed or wrote award settings at delta 0"
fi
# Everything on at once.
if run_render scoring-all TC_SCORING_DRIVER=1 \
  "TC_SECRET_tc_staging_db_gate_driver_password=$gate_driver_ok" \
  TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=5 "TC_NOVELTY_UTILITY_SETTLE_FROM=$settle_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED true
  expect "$out/ingest.env" TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM "$settle_ok"
  no_duplicate_keys "$out/ingest.env" "the full scoring render"
  if grep -q "$gate_driver_ok" "$scratch/scoring-all.out" "$scratch/scoring-all.err"; then
    fail "render-env.sh printed the gate driver password"
  else
    pass "render-env.sh did not print the gate driver password"
  fi
else
  fail "render-env.sh failed with the driver and the award on"
  cat "$scratch/scoring-all.err" >&2
fi

# The scorer on a NEAR AI production direct-completions host (staging
# design 3.9, owner exception 2026-10-24): off by default, and on it renders
# the scorer's base, model, the flag and the scorer's own key, and nothing
# else changes.
scorer_keys='^(TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST|TRACE_COMMONS_NEAR_AI_SCORER_API_KEY)='
scorer_host_ok=https://qwen3-8-27b.completions.near.ai/v1
scorer_model_ok=Qwen/Qwen3.8-27B
scorer_key_ok=sk-dummy-production-scorer-key-7e2a
if ! grep -Eq "$scorer_keys" "$ingest_default"; then
  pass "a default render writes no scorer production-host settings"
else
  fail "a default render wrote scorer production-host settings"
fi
expect "$ingest_default" TRACE_COMMONS_NEAR_AI_BASE_URL https://cloud-stg-api.near.ai/v1
if run_render scorer-production TC_SCORER_PRODUCTION_HOST="$scorer_host_ok" \
  TC_SCORER_MODEL="$scorer_model_ok" \
  "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok"; then
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_BASE_URL "$scorer_host_ok"
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_MODEL "$scorer_model_ok"
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST true
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_SCORER_API_KEY "$scorer_key_ok"
  # Every other NEAR AI setting is staging's, and the production key is in
  # exactly one line.
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_API_KEY dummy-staging-near-ai-key
  expect "$out/ingest.env" TRACE_NEAR_AI_PRIVACY_API_KEY dummy-staging-near-ai-key
  expect "$out/ingest.env" TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL https://cloud-stg-api.near.ai/v1
  expect "$out/ingest.env" TRACE_NEAR_AI_PRIVACY_BASE_URL https://cloud-stg-api.near.ai/v1
  if [ "$(grep -c -F "$scorer_key_ok" "$out/ingest.env")" = 1 ]; then
    pass "the production scorer key is in one line of ingest.env"
  else
    fail "the production scorer key is not in exactly one line of ingest.env"
  fi
  if grep -q -F "$scorer_key_ok" "$out/issuer.env"; then
    fail "the production scorer key reached issuer.env"
  else
    pass "the production scorer key is not in issuer.env"
  fi
  no_duplicate_keys "$out/ingest.env" "the scorer production-host render"
  if grep -q -F "$scorer_key_ok" "$scratch/scorer-production.out" "$scratch/scorer-production.err"; then
    fail "render-env.sh printed the production scorer key"
  else
    pass "render-env.sh did not print the production scorer key"
  fi
else
  fail "render-env.sh failed with the scorer on a production completions host"
  cat "$scratch/scorer-production.err" >&2
fi
scoring_refused scorer-no-secret TC_SCORER_PRODUCTION_HOST="$scorer_host_ok" \
  TC_SCORER_MODEL="$scorer_model_ok"
if grep -q 'tc-staging-scorer-production-nearai-key' "$scratch/scorer-no-secret.err"; then
  pass "the scorer refusal names the scorer production key secret"
else
  fail "the scorer refusal does not name the scorer production key secret"
fi
scoring_refused scorer-bad-secret TC_SCORER_PRODUCTION_HOST="$scorer_host_ok" \
  TC_SCORER_MODEL="$scorer_model_ok" \
  $'TC_SECRET_tc_staging_scorer_production_nearai_key=sk-one\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true'
scoring_refused scorer-no-model TC_SCORER_PRODUCTION_HOST="$scorer_host_ok" \
  "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok"
scoring_refused scorer-no-host TC_SCORER_MODEL="$scorer_model_ok" \
  "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok"
scoring_refused scorer-bad-model TC_SCORER_PRODUCTION_HOST="$scorer_host_ok" \
  "TC_SCORER_MODEL=Qwen 3 \$x" \
  "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok"
scorer_bad_hosts=(
  https://cloud-api.near.ai/v1
  https://cloud-stg-api.near.ai/v1
  http://qwen3-8-27b.completions.near.ai/v1
  https://qwen3-8-27b.completions.near.ai/v1/
  https://qwen3-8-27b.completions.near.ai
  https://qwen3-8-27b.completions.near.ai:443/v1
  https://user@qwen3-8-27b.completions.near.ai/v1
  https://a.b.completions.near.ai/v1
  https://-a.completions.near.ai/v1
  https://qwen3-8-27b.completions.near.ai.evil.example/v1
  "https://qwen3-8-27b.completions.near.ai/v1?x=1"
  https://evil.example/qwen3-8-27b.completions.near.ai/v1
)
for bad in "${scorer_bad_hosts[@]}"; do
  scoring_refused "scorer-bad-host-${bad//[^0-9A-Za-z]/_}" "TC_SCORER_PRODUCTION_HOST=$bad" \
    TC_SCORER_MODEL="$scorer_model_ok" \
    "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok"
done
if grep -q 'TC_SCORER_PRODUCTION_HOST' "$scratch/scorer-bad-host-https___cloud_api_near_ai_v1.err"; then
  pass "the scorer host refusal names TC_SCORER_PRODUCTION_HOST"
else
  fail "the scorer host refusal does not name TC_SCORER_PRODUCTION_HOST"
fi
# Without the host the secret is not read, set or not.
if run_render scorer-off-secret-set \
  "TC_SECRET_tc_staging_scorer_production_nearai_key=$scorer_key_ok" \
  && ! grep -Eq "$scorer_keys" "$out/ingest.env" \
  && ! grep -q -F "$scorer_key_ok" "$out/ingest.env"; then
  pass "a render without TC_SCORER_PRODUCTION_HOST ignores the scorer production key"
else
  fail "a render without TC_SCORER_PRODUCTION_HOST wrote scorer production-host settings"
fi

# Automatic credit settlement is an explicit deployment opt-in, independent
# of on-chain NEAR settlement and without a reusable tenant bearer token.
if run_render settlement-auto TC_CREDIT_SETTLEMENT_ALL_TENANTS=1 \
    TC_CREDIT_SETTLEMENT_POLICY_VERSION=staging-2026-10; then
  expect "$out/ingest.env" TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_SCOPE all_tenants
  expect "$out/ingest.env" TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED true
  expect "$out/ingest.env" TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION staging-2026-10
  expect "$out/ingest.env" TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN false
  if grep -q '^TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN=' "$out/ingest.env"; then
    fail "all-tenant settlement wrote a tenant bearer"
  fi
else
  fail "explicit all-tenant settlement render failed"
fi
if run_render settlement-auto-no-policy TC_CREDIT_SETTLEMENT_ALL_TENANTS=1; then
  fail "all-tenant settlement accepted no policy version"
elif refused settlement-auto-no-policy; then
  pass "all-tenant settlement requires explicit policy version"
fi
if run_render settlement-auto-default; then
  if grep -q '^TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED=true' "$out/ingest.env"; then
    fail "default render enabled automatic settlement"
  else
    pass "default render leaves automatic settlement disabled"
  fi
fi

# Earned client upload trust is staging-only and disabled unless explicitly
# selected. Synthetic JWT-shaped tokens exercise rendering, never token auth.
upload_trust_keys='^TRACE_COMMONS_(CLIENT_UPLOAD_TRUST_(DATABASE_URL|POLICY_(JSON|VERSION))|ACCOUNT_TRUST_(SHADOW_POLICY_(JSON|VERSION)|SCHEDULER_(ENABLED|TOKEN|INTERVAL_SECONDS|LIMIT)))='
if grep -Eq "$upload_trust_keys" "$ingest_default"; then
  fail "the default staging render writes upload trust settings"
else
  pass "the default staging render leaves upload trust disabled"
fi
upload_trust_token_ok=eyJhbGciOiJFZERTQSJ9.eyJyb2xlIjoiYWNjb3VudF90cnVzdF93b3JrZXIifQ.c3ludGhldGljX3NpZ25hdHVyZQ
upload_trust_password_ok=dummystaginguploadtrustpassword
upload_trust_gate_ok=staging-quality-v1
upload_trust_dedup_ok=events.v2+simhash.v2
upload_trust_options=(
  "TC_UPLOAD_TRUST_GATE_POLICY_VERSION=$upload_trust_gate_ok"
  "TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION=$upload_trust_dedup_ok"
  "TC_SECRET_tc_staging_account_trust_worker_token=$upload_trust_token_ok"
  "TC_SECRET_tc_staging_db_upload_trust_password=$upload_trust_password_ok"
)
# This independent literal pins the full policy, including count-once/no-bonus
# controls and no age/week wait, rather than comparing a template to itself.
upload_trust_policy_want='{"version":"staging-earned-upload-v1","processing_cost_bound":1,"bounded_allowance":100,"period":{"mode":"fixed","seconds":86400},"growth_rule":"tiered-v1","growth":{"window_seconds":7257600,"weekly_cap":10000,"attested_unit_weight":1,"require_gate_pass":true,"q_min_micros":null,"credit_quality_calibration_versions":[],"penalty_cooldown_seconds":7776000,"evaluation_max_age_seconds":172800,"allowance_ceiling":500,"evaluator_versions":["staging-quality-v1"],"dedup_signal_versions":["events.v2+simhash.v2"],"tiers":[{"units":0,"active_weeks":0,"age_seconds":0,"attested_units":0,"multiplier":1},{"units":5,"active_weeks":0,"age_seconds":0,"attested_units":0,"multiplier":2},{"units":20,"active_weeks":0,"age_seconds":0,"attested_units":0,"multiplier":3},{"units":50,"active_weeks":0,"age_seconds":0,"attested_units":0,"multiplier":5}]}}'
for mode in shadow applied; do
  if RENDER_XTRACE=1 run_render "upload-trust-$mode" \
    "TC_CLIENT_UPLOAD_TRUST_MODE=$mode" "${upload_trust_options[@]}"; then
    if [ "$mode" = shadow ]; then
      policy_prefix=TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY
      absent_prefix=TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY
    else
      policy_prefix=TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY
      absent_prefix=TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY
    fi
    expect "$out/ingest.env" "${policy_prefix}_VERSION" staging-earned-upload-v1
    expect "$out/ingest.env" "${policy_prefix}_JSON" "$upload_trust_policy_want"
    expect "$out/ingest.env" TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL \
      "postgres://tc_upload_trust_login:$upload_trust_password_ok@127.0.0.1:5432/tracecommons"
    expect "$out/ingest.env" DATABASE_URL \
      "postgres://app:dummystagingdbpassword@127.0.0.1:5432/tracecommons"
    expect "$out/ingest.env" TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED true
    expect "$out/ingest.env" TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS 300
    expect "$out/ingest.env" TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT 10000
    expect "$out/ingest.env" TRACE_COMMONS_ADMISSION_ENABLED false
    expect "$out/ingest.env" TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED false
    no_duplicate_keys "$out/ingest.env" "the $mode upload trust render"
    if grep -q "^${absent_prefix}_" "$out/ingest.env" \
      || grep -q '^TRACE_COMMONS_WITNESS_' "$out/ingest.env"; then
      fail "the $mode upload trust render writes another trust mode or witness settings"
    else
      pass "the $mode upload trust render keeps the other mode and witness unset"
    fi
    if [ "$(value_of "$out/ingest.env" TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN)" = "$upload_trust_token_ok" ] \
      && [ "$(grep -c -F "$upload_trust_token_ok" "$out/ingest.env")" = 1 ] \
      && ! grep -q -F "$upload_trust_token_ok" "$out/issuer.env"; then
      pass "the upload trust worker token is assigned once to ingest alone"
    else
      fail "the upload trust worker token was not confined to its ingest setting"
    fi
    if [ "$(grep -c -F "$upload_trust_password_ok" "$out/ingest.env")" = 1 ] \
      && ! grep -q -F "$upload_trust_password_ok" "$out/issuer.env"; then
      pass "the upload trust database password is confined to its dedicated ingest setting"
    else
      fail "the upload trust database password escaped its dedicated setting"
    fi
    for secret_value in "$upload_trust_token_ok" "$upload_trust_password_ok"; do
      if grep -q -F "$secret_value" "$scratch/upload-trust-$mode.out" "$scratch/upload-trust-$mode.err"; then
        fail "the $mode upload trust render leaked a trust secret under bash -x"
      else
        pass "the $mode upload trust render keeps a trust secret out of stdout and xtrace"
      fi
    done
  else
    fail "the $mode upload trust render failed"
    cat "$scratch/upload-trust-$mode.err" >&2
  fi
done
upload_trust_refused() {
  local stem="upload-trust-$1"
  shift
  if run_render "$stem" "$@"; then
    fail "the upload trust render accepted $stem"
  elif ! refused "$stem"; then
    fail "the upload trust render failed without a refusal ($stem)"
  elif [ -e "$out/ingest.env" ] || [ -e "$out/issuer.env" ]; then
    fail "the refused upload trust render wrote env files ($stem)"
  else
    pass "the upload trust render refuses $stem without writing env files"
  fi
}
upload_trust_refused bad-mode TC_CLIENT_UPLOAD_TRUST_MODE=enabled "${upload_trust_options[@]}"
upload_trust_refused labels-without-mode "${upload_trust_options[@]}"
upload_trust_refused no-labels TC_CLIENT_UPLOAD_TRUST_MODE=shadow
upload_trust_refused gate-only TC_CLIENT_UPLOAD_TRUST_MODE=applied \
  "TC_UPLOAD_TRUST_GATE_POLICY_VERSION=$upload_trust_gate_ok"
upload_trust_refused dedup-only TC_CLIENT_UPLOAD_TRUST_MODE=applied \
  "TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION=$upload_trust_dedup_ok"
for bad in 'has space' 'has/slash' 'has:colon' 'has"quote' 'has$dollar' \
  $'new\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' \
  "$(printf 'x%.0s' $(seq 65))"; do
  upload_trust_refused bad-gate "${upload_trust_options[@]}" TC_CLIENT_UPLOAD_TRUST_MODE=applied \
    "TC_UPLOAD_TRUST_GATE_POLICY_VERSION=$bad"
  upload_trust_refused bad-dedup "${upload_trust_options[@]}" TC_CLIENT_UPLOAD_TRUST_MODE=shadow \
    "TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION=$bad"
done
for bad in '' not-a-jwt 'one.two' 'one.two.three.four' 'one.two.three with-space' \
  $'one.two.three\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' $'one.two.three\r'; do
  upload_trust_refused bad-token "${upload_trust_options[@]}" TC_CLIENT_UPLOAD_TRUST_MODE=applied \
    "TC_SECRET_tc_staging_account_trust_worker_token=$bad"
done
for bad in '' short 'pass@word/host' 'pass:word' 'password with spaces' \
  $'validlongpassword\nTRACE_COMMONS_WITNESS_BYPASS_ENABLED=true' $'validlongpassword\r'; do
  upload_trust_refused bad-password "${upload_trust_options[@]}" TC_CLIENT_UPLOAD_TRUST_MODE=shadow \
    "TC_SECRET_tc_staging_db_upload_trust_password=$bad"
done
# Disabled mode must not read either optional secret, even if malformed.
if run_render upload-trust-disabled-secret \
  $'TC_SECRET_tc_staging_account_trust_worker_token=not-a-jwt\nsecond-line' \
  $'TC_SECRET_tc_staging_db_upload_trust_password=bad/password\nsecond-line' \
  && cmp -s "$out/ingest.env" "$ingest_default"; then
  pass "disabled upload trust ignores its secret and leaves the default env byte-identical"
else
  fail "disabled upload trust read its secret or changed the default env"
fi

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
