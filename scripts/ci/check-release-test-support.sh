#!/usr/bin/env bash
# Refuse a shipped build that could compile the ingest crate's `test-support`
# feature.
#
# That feature exports the ingest service's test fixtures
# (`trace_commons_ingest::test_support`) to test packages outside the crate.
# With it on, three levers that are otherwise `cfg(test)`-only exist in the
# library: the field that redirects NEAR AI login introspection (where a
# contributor's NEAR AI access token is sent), the NEAR access-key checker
# override, and the hooks that reset and reconfigure the submit rate limiter.
# None of them is wired to configuration, but a shipped binary must not carry
# them at all. The feature is enabled only by test dev-dependencies, which
# cargo never activates for `cargo build`. Nothing but this check stops a
# later edit from enabling it in a release: a `--features` flag on a build
# or deploy line, an `--all-features` build, a crate turning it on in its
# `[dependencies]`, or the feature entering `default`. A `compile_error!` on
# `--release` is not an option: it would break `cargo test --release`.
#
# Modelled on scripts/ci/check-release-credential-store.sh and
# scripts/ci/check-release-staging-feature.sh.
#
# Usage:
#   scripts/ci/check-release-test-support.sh check [ROOT]
#       Scan the release, packaging and deploy files and every manifest.
#       Exit 1 naming each hit.
#   scripts/ci/check-release-test-support.sh self-test
#       Copy the scanned files to a scratch tree, prove the clean copy passes,
#       then prove each seeded violation fails.
set -euo pipefail

FEATURE="test-support"

# Files that build, package or deploy a shipped server binary. Listed, not
# discovered; a missing file is an error, so a rename cannot drop one out of
# the scan. release-*.yml is also globbed.
BUILD_FILES=(
  .github/workflows/release-apps.yml
  .github/workflows/release-contributor.yml
  .github/workflows/witness-image.yml
  cloudbuild.yaml                             # the hosted ingest and issuer binaries
  deploy/staging-gcp/cloudbuild.yaml          # the staging server binaries
  deploy/pilot-gcp/deploy.sh
  deploy/pilot-gcp/pull-and-install.sh
  deploy/witness/Dockerfile
  scripts/operator/pipeline_tooling/promote.py
)

INGEST_MANIFEST=crates/trace-commons-ingest/Cargo.toml
# Test-only packages that may enable the feature, each in `[dev-dependencies]`
# and by this exact line. Each must stay unpublished and target-less apart
# from its tests, which check 4 holds it to.
TEST_PACKAGES=(
  crates/trace-commons-ingest-contributor-tests/Cargo.toml
)
ENABLING_LINE='trace-commons-ingest={path="../trace-commons-ingest",features=["test-support"]}'

manifests() {
  local m
  for m in Cargo.toml crates/*/Cargo.toml tauri-desktop/src-tauri/Cargo.toml; do
    [[ -f "$m" ]] && printf '%s\n' "$m"
  done
}

check() {
  local root="${1:-.}" failures=0 file hit
  cd "$root"

  local files=("${BUILD_FILES[@]}")
  for file in .github/workflows/release-*.yml; do
    [[ -e "$file" ]] || continue
    [[ " ${files[*]} " == *" $file "* ]] || files+=("$file")
  done
  for file in "${files[@]}" "$INGEST_MANIFEST" "${TEST_PACKAGES[@]}"; do
    if [[ ! -f "$file" ]]; then
      echo "MISSING: $file is on the scan list but does not exist." >&2
      echo "  Update scripts/ci/check-release-test-support.sh if it moved." >&2
      failures=$((failures + 1))
    fi
  done

  # 1. No release, packaging or deploy file names the feature.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file:$hit names the test-only '$FEATURE' feature in a release build path." >&2
      failures=$((failures + 1))
    done < <(grep -n -- "$FEATURE" "$file" || true)
  done

  # 2. No build command in those files uses --all-features, which enables
  #    it. Continuation lines are joined first.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: a build command uses --all-features, which enables '$FEATURE': $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '{
          line = $0
          if (sub(/[\\`][ \t]*$/, "", line)) { buf = buf line " "; next }
          print buf line; buf = ""
        } END { if (buf != "") print buf }' "$file" |
        grep -E -- '--all-features' |
        grep -E 'cargo[^#]*[[:space:]"](build|rustc|install)' || true
    )
  done

  # 3. Every manifest. Permitted: the ingest crate's own `[features]`
  #    definition, and the exact dev-dependency line of a listed test
  #    package. Anything else -- another crate or the workspace root
  #    enabling it, a normal dependency, or `default = [...]` -- ships it.
  local manifest
  while IFS= read -r manifest; do
    local allowed=0 pkg
    for pkg in "${TEST_PACKAGES[@]}"; do [[ "$manifest" == "$pkg" ]] && allowed=1; done
    while IFS= read -r hit; do
      echo "REFUSING: $manifest: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v feature="$FEATURE" -v file="$manifest" -v ingest="$INGEST_MANIFEST" \
        -v allowed="$allowed" -v enabling="$ENABLING_LINE" '
        /^[[:space:]]*\[/ { section = $0; gsub(/[[:space:]]/, "", section) }
        index($0, feature) == 0 { next }
        /^[[:space:]]*#/ { next }
        {
          line = $0; gsub(/[[:space:]]/, "", line)
          if (file == ingest && section == "[features]" && line == feature "=[\"dep:tower\"]") next
          if (allowed == 1 && section == "[dev-dependencies]" && line == enabling) next
          printf "line %d in %s enables the test-only feature outside a listed test dev-dependency: %s\n", NR, section, $0
        }' "$manifest"
    )
  done < <(manifests)

  # 4. A package allowed to enable it is a test package and nothing else:
  #    unpublished, with no library, binary or normal dependencies that a
  #    shipped crate could come to depend on.
  for file in "${TEST_PACKAGES[@]}"; do
    [[ -f "$file" ]] || continue
    if ! grep -qE '^publish[[:space:]]*=[[:space:]]*false[[:space:]]*$' "$file"; then
      echo "REFUSING: $file enables '$FEATURE' and is not 'publish = false'." >&2
      failures=$((failures + 1))
    fi
    while IFS= read -r hit; do
      echo "REFUSING: $file:$hit: a package that enables '$FEATURE' may have test targets and dev-dependencies only." >&2
      failures=$((failures + 1))
    done < <(grep -nE '^[[:space:]]*\[(lib|\[bin\]|dependencies|build-dependencies|target\..*\.dependencies)\]' "$file" || true)
  done

  if ((failures > 0)); then
    echo "$failures release test-support violation(s)." >&2
    return 1
  fi
  echo "no release, packaging or deploy build can enable '$FEATURE'"
}

self_test() {
  local repo scratch status=0
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN

  local file
  for file in "${BUILD_FILES[@]}" "$repo"/.github/workflows/release-*.yml \
    "$repo"/Cargo.toml "$repo"/crates/*/Cargo.toml "$repo"/tauri-desktop/src-tauri/Cargo.toml; do
    file="${file#"$repo"/}"
    mkdir -p "$scratch/$(dirname "$file")"
    cp "$repo/$file" "$scratch/$file"
  done

  if ! (check "$scratch" >/dev/null 2>&1); then
    echo "self-test: the unmodified tree should pass and did not" >&2
    (check "$scratch") || true
    return 1
  fi

  # Each case: a description, a file, and a perl substitution seeding one
  # violation.
  local cases=(
    "--features on the hosted ingest build|cloudbuild.yaml|s/\\z/\\n# cargo build --release -p trace-commons-ingest --features $FEATURE\\n/"
    "--all-features on a deploy build|deploy/pilot-gcp/deploy.sh|s/\\z/\\ncargo build --release -p trace-commons-ingest --all-features\\n/"
    "--all-features on a continuation line|deploy/pilot-gcp/deploy.sh|s/\\z/\\ncargo build --release -p trace-commons-ingest \\\\\\n  --all-features\\n/"
    "the promote tooling naming it|scripts/operator/pipeline_tooling/promote.py|s/near-ai-scorer,gcs-client,gcp-kms/near-ai-scorer,gcs-client,gcp-kms,$FEATURE/"
    "the feature entering default|crates/trace-commons-ingest/Cargo.toml|s/^default = \\[\\]\$/default = [\"$FEATURE\"]/m"
    "the ingest crate enabling it on itself in normal dependencies|crates/trace-commons-ingest/Cargo.toml|s/^(anyhow = \"1\")/\$1\\ntrace-commons-ingest = { path = \".\", features = [\"$FEATURE\"] }/m"
    "the server crate enabling it|crates/trace-commons-server/Cargo.toml|s/\\z/\\n[dev-dependencies.seeded]\\nfeatures = [\"$FEATURE\"]\\n/"
    "the workspace root enabling it|Cargo.toml|s/\\z/\\n[workspace.dependencies.seeded]\\nfeatures = [\"$FEATURE\"]\\n/"
    "the test package gaining a normal dependency|crates/trace-commons-ingest-contributor-tests/Cargo.toml|s/\\z/\\n[dependencies]\\nanyhow = \"1\"\\n/"
    "the test package becoming publishable|crates/trace-commons-ingest-contributor-tests/Cargo.toml|s/^publish = false\$/publish = true/m"
    "the test package enabling it outside dev-dependencies|crates/trace-commons-ingest-contributor-tests/Cargo.toml|s/^\\[dev-dependencies\\]\$/[build-dependencies]/m"
  )
  local entry desc target program before
  for entry in "${cases[@]}"; do
    IFS='|' read -r desc target program <<<"$entry"
    before="$(cat "$scratch/$target")"
    perl -0pi -e "$program" "$scratch/$target"
    if [[ "$(cat "$scratch/$target")" == "$before" ]]; then
      echo "self-test: seeding '$desc' changed nothing in $target; the fixture drifted" >&2
      status=1
    elif (check "$scratch" >/dev/null 2>&1); then
      echo "self-test: '$desc' was NOT refused" >&2
      status=1
    else
      echo "self-test: refused as expected: $desc"
    fi
    printf '%s\n' "$before" >"$scratch/$target"
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
