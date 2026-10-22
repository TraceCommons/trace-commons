#!/usr/bin/env bash
# Refuse a shipped build that could compile the `near-ai-staging` feature.
#
# That feature switches the compiled-in NEAR AI host to NEAR AI staging: in
# the server and ingest (login introspection and the NEAR AI payout rail) and
# in the contributor daemon (sign-in and org listing). It exists for one
# operator test run against staging, which needs an optimized build with the
# feature on, so a `compile_error!` on `--release` is not an option. Nothing
# but this check stops a later edit from shipping it: a `--features` flag on
# a release or deploy build line, an `--all-features` or `--workspace` build,
# a crate turning it on in its `[dependencies]`, or the feature entering
# `default`.
#
# Modelled on scripts/ci/check-release-credential-store.sh, with a wider scan:
# this feature reaches the hosted server's build path too (cloudbuild.yaml,
# the promote tooling, the pilot deploy scripts), not only the client apps.
#
# Usage:
#   scripts/ci/check-release-staging-feature.sh check [ROOT]
#       Scan the release, packaging and deploy files and the manifests of
#       every crate that ships. Exit 1 naming each hit.
#   scripts/ci/check-release-staging-feature.sh self-test
#       Copy the scanned files to a scratch tree, prove the clean copy passes,
#       then prove each seeded violation fails. Keeps the check from rotting
#       into one that can no longer fail.
set -euo pipefail

FEATURE="near-ai-staging"

# Files that build, package or deploy a shipped artifact. Listed, not
# discovered, because a release step can call any script; a missing file is
# an error, so a rename cannot silently drop one out of the scan.
# release-*.yml is ALSO globbed, so a new release workflow is covered without
# editing this list.
BUILD_FILES=(
  .github/workflows/release-apps.yml          # macOS DMG, FFI dylib/DLL, MSIX, appinstaller, flatpak
  .github/workflows/release-contributor.yml   # contributor CLI release binaries
  .github/workflows/witness-image.yml         # witness container image
  cloudbuild.yaml                             # the hosted ingest and issuer binaries
  deploy/pilot-gcp/deploy.sh
  deploy/pilot-gcp/pull-and-install.sh
  deploy/witness/Dockerfile
  scripts/operator/pipeline_tooling/promote.py
  macos/scripts/make-app-bundle.sh
  macos/scripts/make-release-dmg.sh
  windows/scripts/make-msix.ps1
  windows/scripts/make-appinstaller.ps1
  windows/src/TraceCommons.App/TraceCommons.App.csproj
  scripts/flatpak/build-and-sign.sh
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
  scripts/linux-build.sh
  scripts/linux-build.Dockerfile
  tauri-desktop/scripts/build.sh
  tauri-desktop/src-tauri/tauri.conf.json
)

# Crates that ship. A normal dependency that names the feature turns it on in
# every build of that crate, release included.
SHIPPED_MANIFESTS=(
  crates/trace-commons-server/Cargo.toml
  crates/trace-commons-ingest/Cargo.toml
  crates/trace-commons-contributor/Cargo.toml
  crates/trace-commons-contributor-sources/Cargo.toml
  crates/trace-commons-contributor-insights/Cargo.toml
  crates/trace-commons-contributor-platform/Cargo.toml
  crates/trace-commons-contributor-ffi/Cargo.toml
  crates/trace-commons-contributor-gtk/Cargo.toml
  tauri-desktop/src-tauri/Cargo.toml
)

check() {
  local root="${1:-.}" failures=0 file
  cd "$root"

  local files=("${BUILD_FILES[@]}")
  for file in .github/workflows/release-*.yml; do
    [[ -e "$file" ]] || continue
    [[ " ${files[*]} " == *" $file "* ]] || files+=("$file")
  done

  for file in "${files[@]}" "${SHIPPED_MANIFESTS[@]}"; do
    if [[ ! -f "$file" ]]; then
      echo "MISSING: $file is on the release scan list but does not exist." >&2
      echo "  Update scripts/ci/check-release-staging-feature.sh if it moved." >&2
      failures=$((failures + 1))
    fi
  done

  # 1. No release, packaging or deploy file names the feature at all.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file:$hit names the '$FEATURE' feature in a release build path." >&2
      failures=$((failures + 1))
    done < <(grep -n -- "$FEATURE" "$file" || true)
  done

  # 2. No build command in those files uses --all-features, which would
  #    enable it, or --workspace (--all), which unifies features across every
  #    member, so any member that enables the feature -- not only the shipped
  #    crates scanned in 3 -- would reach the build. Continuation lines (trailing \ or PowerShell `) are joined
  #    first so a flag on the next line of the same command is still seen.
  #    Only build commands count: cargo-deny's `--all-features` is an audit.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: a build command uses --all-features or --workspace, which can enable '$FEATURE': $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '{
          line = $0
          if (sub(/[\\`][ \t]*$/, "", line)) { buf = buf line " "; next }
          print buf line; buf = ""
        } END { if (buf != "") print buf }' "$file" |
        grep -E -- '--all-features|--(workspace|all)([[:space:]]|$)' |
        grep -E '(cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?(build|rustc|install|tauri)|tauri[^[:space:]]*[[:space:]]+build)' || true
    )
  done

  # 3. Manifests of shipped crates. The only permitted mentions are the exact
  #    feature definitions under [features]: an empty one in the server and
  #    contributor crates, and ingest forwarding to the server. Anything else
  #    -- a dependency enabling it, `default = [..., "near-ai-staging"]`, or a
  #    definition that forwards somewhere new -- ships it or widens it.
  for file in "${SHIPPED_MANIFESTS[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v feature="$FEATURE" -v file="$file" '
        /^[[:space:]]*\[/ { section = $0; gsub(/[[:space:]]/, "", section) }
        index($0, feature) == 0 { next }
        /^[[:space:]]*#/ { next }
        {
          line = $0; gsub(/[[:space:]]/, "", line)
          if (section == "[features]") {
            if (file ~ /trace-commons-server\/Cargo.toml$/ && line == feature "=[]") next
            if (file ~ /trace-commons-contributor\/Cargo.toml$/ && line == feature "=[]") next
            if (file ~ /trace-commons-ingest\/Cargo.toml$/ && line == feature "=[\"trace-commons-server/" feature "\"]") next
          }
          printf "line %d in %s enables the staging feature outside its own definition: %s\n", NR, section, $0
        }' "$file"
    )
  done

  if ((failures > 0)); then
    echo "$failures release staging-feature violation(s)." >&2
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
  for file in "${BUILD_FILES[@]}" "${SHIPPED_MANIFESTS[@]}" "$repo"/.github/workflows/release-*.yml; do
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
  # violation. perl rather than sed: BSD sed has no `\n` in a replacement,
  # which would quietly turn the continuation-line case into a one-line one.
  local cases=(
    "--features on the hosted ingest build|cloudbuild.yaml|s/--features gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral/\$&,$FEATURE/"
    "the promote tooling building it|scripts/operator/pipeline_tooling/promote.py|s/near-ai-scorer,gcs-client,gcp-kms/\$&,$FEATURE/"
    "the pilot deploy notes requiring it|deploy/pilot-gcp/deploy.sh|s/#   --features gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral/\$&,$FEATURE/"
    "--features on the contributor release build|.github/workflows/release-contributor.yml|s/cargo build --locked --release --bin trace-commons-contributor/\$& --features trace-commons-contributor\/$FEATURE/"
    "--all-features on the FFI dylib build|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& --all-features/"
    "--workspace on the FFI dylib build|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& --workspace/"
    "--all-features on a continuation line of the ingest build|cloudbuild.yaml|s/cargo build --release -p trace-commons-ingest --bin trace-commons-ingest/\$& \\\\\n          --all-features/"
    "the feature entering the server's default|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "the feature entering the ingest default|crates/trace-commons-ingest/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "the feature entering the contributor's default|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "ingest enabling it on its server dependency|crates/trace-commons-ingest/Cargo.toml|s#^trace-commons-server = \\{ path = \"../trace-commons-server\" \\}#trace-commons-server = { path = \"../trace-commons-server\", features = [\"$FEATURE\"] }#m"
    "the FFI enabling it on the contributor|crates/trace-commons-contributor-ffi/Cargo.toml|s#^(trace-commons-contributor = \\{[^}\\n]*)\\}#\$1, features = [\"$FEATURE\"] }#m"
    "the server definition forwarding somewhere new|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\\]/$FEATURE = [\"trace-commons-gate-enclave\\/near-ai-scorer\"]/m"
    "a flatpak build naming it|crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml|s/\\A/# $FEATURE\\n/"
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
