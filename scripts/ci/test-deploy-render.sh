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
done <"$fixtures/dummy.env"

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
  if cmp -s "$scratch/pilot-$name" "$fixtures/$name"; then
    pass "pilot $name renders byte-identical"
  else
    fail "pilot $name render differs from the fixture"
    diff "$fixtures/$name" "$scratch/pilot-$name" >&2 || true
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

secret_env=(
  TC_SECRET_tc_staging_nearai_inference_key=dummy-staging-near-ai-key
  TC_SECRET_tc_staging_db_app_password=dummy-staging-db-password
  TC_SECRET_tc_staging_db_login_resolver_password=dummy-staging-resolver-password
  TC_SECRET_tc_staging_gate_master_key=dummy-staging-gate-master-key-0123456789abcdef
  TC_SECRET_tc_staging_artifact_key=dummy-staging-artifact-key-0123456789abcdef0123
  TC_SECRET_tc_staging_near_account_index_pepper=ZHVtbXktc3RhZ2luZy1wZXBwZXItMDEyMzQ1Njc4OWFiY2RlZg==
  TC_SECRET_tc_staging_near_account_name_kek_key=dummy-staging-name-kek-0123456789abcdef0123456789
  "TC_SECRET_tc_staging_issuer_signing_keypair=$issuer_keypair"
  "TC_SECRET_tc_staging_issuer_workload_public_key=$pem_public"
  "TC_SECRET_tc_staging_ingest_attestation_keypair=$attestation"
)

run_render() {
  # $1: output stem; the rest: NAME=value overrides.
  local stem="$1"
  shift
  rm -rf "$out" "$site" "$deploy_dir"
  mkdir -p "$out" "$site" "$deploy_dir"
  env -i PATH="/usr/bin:/bin:$(dirname "$envsubst_bin")" HOME="$scratch/home" \
    TC_RENDER_OUT_DIR="$out" TC_SITE_ROOT="$site" TC_DEPLOY_DIR="$deploy_dir" \
    TC_RENDER_SKIP_CHOWN=1 TC_LE_EMAIL=ops@example.invalid TC_GCP_REGION=us-central1 \
    "${secret_env[@]}" "$@" \
    bash "$render" --secrets-from-env >"$scratch/$stem.out" 2>"$scratch/$stem.err"
}

if run_render render; then
  pass "render-env.sh renders with dummy secrets"
else
  fail "render-env.sh failed with dummy secrets"
  cat "$scratch/render.err" >&2
fi
ingest="$out/ingest.env"
issuer="$out/issuer.env"

value_of() {
  # The value of one assignment in a rendered env file, quotes as written.
  grep -E "^$2=" "$1" | tail -n 1 | sed "s/^$2=//"
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
  expect "$ingest" TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY dummy-staging-name-kek-0123456789abcdef0123456789
  expect "$ingest" TRACE_COMMONS_ADMISSION_ENABLED false
  expect "$ingest" TRACE_COMMONS_NEAR_AI_PAYOUT_MODE disabled
  expect "$ingest" TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED false
  expect "$ingest" TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS 0
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_ID "$H"
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_ORIGIN "https://$H,https://ingest.$H"
  expect "$ingest" TRACE_COMMONS_WEBAUTHN_RP_NAME '"Near.ai Staging"'
  expect "$ingest" TRACE_COMMONS_SIGNED_TOKEN_ISSUER "https://issuer.$H"
  expect "$ingest" TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS "issuer.$H"
  expect "$ingest" DATABASE_URL postgres://app:dummy-staging-db-password@127.0.0.1:5432/tracecommons
  expect "$ingest" TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL postgres://tc_login_resolver_login:dummy-staging-resolver-password@127.0.0.1:5432/tracecommons
  expect "$ingest" TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME projects/tracecommons-pilot-2026/locations/us-central1/keyRings/tc-staging/cryptoKeys/kek-v1
  expect "$ingest" TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET tc-staging-artifacts-20261021
  expect "$ingest" TRACE_COMMONS_REMOTE_OBJECT_STORE_CREDENTIAL_REF tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com
  expect "$ingest" TRACE_COMMONS_GATE_SERVICE_MASTER_KEY dummy-staging-gate-master-key-0123456789abcdef
  expect "$ingest" TRACE_COMMONS_ARTIFACT_KEY_HEX dummy-staging-artifact-key-0123456789abcdef0123
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
  # tracecommons.ai left over is a pilot host. Comment lines are left out:
  # the Caddyfile keeps the pilot template's comments, which name examples.
  stripped="$(cat "$ingest" "$issuer" "$deploy_dir/Caddyfile" 2>/dev/null \
    | grep -v -E '^[[:space:]]*#' | sed 's/staging\.tracecommons\.ai//g')"
  for needle in tc-pilot qwen3-6-35b cloud-api.near.ai completions.near.ai nip.io \
    tracecommons.ai '/trace-commons$' 'pilot-workload-issuer' '\$\{'; do
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
  expect "$issuer" DATABASE_URL postgres://app:dummy-staging-db-password@127.0.0.1:5432/tracecommons
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
if grep -q -F -e dummy-staging -e 'PRIVATE KEY' "$scratch/render.out" "$scratch/render.err"; then
  fail "render-env.sh printed a secret value"
else
  pass "render-env.sh prints no secret value"
fi

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

caddyfile="$deploy_dir/Caddyfile"
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

# Expected measurements go in only when given.
if run_render measured TC_NEAR_AI_EXPECTED_MEASUREMENTS="mrtd=$(printf '%096d' 0)"; then
  expect "$ingest" TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS "mrtd=$(printf '%096d' 0)"
else
  fail "render-env.sh failed with expected measurements"
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

# 4. caddy adapt, where caddy is installed.
if command -v caddy >/dev/null 2>&1; then
  for file in "$scratch/pilot-Caddyfile" "$caddyfile"; do
    [ -f "$file" ] || continue
    if caddy adapt --config "$file" --adapter caddyfile >/dev/null 2>"$scratch/adapt.err"; then
      pass "caddy adapt accepts $(basename "$(dirname "$file")")/$(basename "$file")"
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
