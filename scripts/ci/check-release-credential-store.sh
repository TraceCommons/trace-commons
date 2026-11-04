#!/usr/bin/env bash
# Refuse a shipped build that could compile a test-only feature of the
# contributor crates: `test-credential-store` or the FFI crate's
# `test-keep-panic-messages` (see the second half of this header).
#
# `test-credential-store`:
#
# That feature replaces the OS credential store (device key, account session,
# Private AI credentials) with a PLAINTEXT file in the config directory. It
# exists so `cargo test` stops writing the developer's real login keychain, and
# it is enabled only by test dev-dependencies (the contributor's self edge
# and the unpublished client/server test package), which cargo never activates
# for `cargo build`/`--release`. Nothing but this
# check stops a later edit from enabling it in a release: a `--features` flag
# on a release build line, an `--all-features` build, a shell crate turning it
# on in its `[dependencies]`, or the feature entering `default`. A
# `compile_error!` is not an option -- it would break `cargo test --release`.
#
# `test-keep-panic-messages` (FFI crate) makes `start_daemon_handle` skip
# installing the daemon's location-only panic hook, the privacy control that
# keeps panic messages, which can quote prompt text, out of stderr and the
# on-disk logs. A release or staging build with it on ships without that
# control. It is enabled only by the FFI crate's self dev-dependency. The same
# three routes are refused: naming it in a release/staging build file, a
# build command with --all-features, and any crate or workspace manifest
# enabling it other than that self dev-dependency (and its [features] line).
#
# Usage:
#   scripts/ci/check-release-credential-store.sh check [ROOT]
#       Scan the release/packaging workflows, the build scripts they invoke,
#       and the manifests of every crate that ships. Exit 1 naming each hit.
#   scripts/ci/check-release-credential-store.sh self-test
#       Copy the scanned files to a scratch tree, prove the clean copy passes,
#       then prove each seeded violation fails. Keeps the check from rotting
#       into one that can no longer fail.
set -euo pipefail

FEATURE="test-credential-store"
PANIC_FEATURE="test-keep-panic-messages"

# Files that build or package a shipped artifact. Listed, not discovered,
# because a release step can call any script; a missing file is an error, so a
# rename cannot silently drop one out of the scan. release-*.yml is ALSO
# globbed, so a new release workflow is covered without editing this list.
BUILD_FILES=(
  .github/workflows/release-apps.yml          # macOS DMG, FFI dylib/DLL, MSIX, appinstaller, flatpak
  .github/workflows/release-contributor.yml   # contributor CLI release binaries
  .github/workflows/staging-macos.yml         # the staging macOS app
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
  crates/trace-commons-contributor/Cargo.toml
  crates/trace-commons-contributor-sources/Cargo.toml
  crates/trace-commons-contributor-insights/Cargo.toml
  crates/trace-commons-contributor-platform/Cargo.toml
  crates/trace-commons-contributor-ffi/Cargo.toml
  crates/trace-commons-contributor-gtk/Cargo.toml
  tauri-desktop/src-tauri/Cargo.toml
)

# Every manifest that could enable PANIC_FEATURE: the workspace root and every
# crate, not only the shipped ones, because a workspace-level or helper-crate
# edit reaches a release build through feature unification. Globbed so a new
# crate is covered without editing this script.
panic_manifests() {
  local m
  for m in Cargo.toml crates/*/Cargo.toml tauri-desktop/src-tauri/Cargo.toml; do
    [[ -f "$m" ]] && printf '%s\n' "$m"
  done
}

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
      echo "  Update scripts/ci/check-release-credential-store.sh if it moved." >&2
      failures=$((failures + 1))
    fi
  done

  # 1. No release/packaging file names either feature at all.
  local feature
  for feature in "$FEATURE" "$PANIC_FEATURE"; do
    for file in "${files[@]}"; do
      [[ -f "$file" ]] || continue
      while IFS= read -r hit; do
        echo "REFUSING: $file:$hit names the test-only '$feature' feature in a release build path." >&2
        failures=$((failures + 1))
      done < <(grep -n -- "$feature" "$file" || true)
    done
  done

  # 2. No build command in those files uses --all-features, which would
  #    enable it. Continuation lines (trailing \ or PowerShell `) are joined
  #    first so a flag on the next line of the same command is still seen.
  #    Only build commands count: cargo-deny's `--all-features` is an audit.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: a build command uses --all-features, which enables '$FEATURE' and '$PANIC_FEATURE': $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '{
          line = $0
          if (sub(/[\\`][ \t]*$/, "", line)) { buf = buf line " "; next }
          print buf line; buf = ""
        } END { if (buf != "") print buf }' "$file" |
        grep -E -- '--all-features' |
        grep -E '(cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?(build|rustc|install|tauri)|tauri[^[:space:]]*[[:space:]]+build)' || true
    )
  done

  # 3. Manifests of shipped crates. The only permitted mentions are the
  #    exact feature definitions/forwarding under [features] and the self
  #    dev-dependency in trace-commons-contributor. Anything else -- a normal
  #    dependency enabling it, or `default = [..., "test-credential-store"]`
  #    -- ships it.
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
          if (file ~ /trace-commons-contributor\/Cargo.toml$/) {
            if (section == "[features]" && line == feature "=[\"trace-commons-contributor-insights/" feature "\",\"trace-commons-contributor-platform/" feature "\"]") next
            if (section ~ /dev-dependencies\]$/ && line ~ /^trace-commons-contributor=\{path="\."/) next
          }
          if (file ~ /trace-commons-contributor-insights\/Cargo.toml$/ && section == "[features]" && line == feature "=[\"trace-commons-contributor-platform/" feature "\"]") next
          if (file ~ /trace-commons-contributor-platform\/Cargo.toml$/ && section == "[features]" && line == feature "=[]") next
          printf "line %d in %s enables the test-only feature outside the self dev-dependency: %s\n", NR, section, $0
        }' "$file"
    )
  done

  # 4. PANIC_FEATURE in any manifest. Permitted: the FFI crate's own
  #    [features] definition and its self dev-dependency, which cargo applies
  #    to tests only. Anything else -- another crate or the workspace root
  #    enabling it, a normal dependency, or `default = [...]` -- ships it.
  local manifest
  while IFS= read -r manifest; do
    while IFS= read -r hit; do
      echo "REFUSING: $manifest: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v feature="$PANIC_FEATURE" -v file="$manifest" '
        /^[[:space:]]*\[/ { section = $0; gsub(/[[:space:]]/, "", section) }
        index($0, feature) == 0 { next }
        /^[[:space:]]*#/ { next }
        {
          line = $0; gsub(/[[:space:]]/, "", line)
          if (file ~ /trace-commons-contributor-ffi\/Cargo.toml$/) {
            if (section == "[features]" && line == feature "=[]") next
            if (section == "[dev-dependencies]" && line == "trace-commons-contributor-ffi={path=\".\",features=[\"" feature "\"]}") next
          }
          printf "line %d in %s enables the test-only feature outside the FFI self dev-dependency: %s\n", NR, section, $0
        }' "$manifest"
    )
  done < <(panic_manifests)

  if ((failures > 0)); then
    echo "$failures release test-only-feature violation(s)." >&2
    return 1
  fi
  echo "no release or packaging build can enable '$FEATURE' or '$PANIC_FEATURE'"
}

self_test() {
  local repo scratch status=0
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN

  local file
  for file in "${BUILD_FILES[@]}" "${SHIPPED_MANIFESTS[@]}" "$repo"/.github/workflows/release-*.yml \
    "$repo"/Cargo.toml "$repo"/crates/*/Cargo.toml; do
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
    "--features on the contributor release build|.github/workflows/release-contributor.yml|s/cargo build --locked --release --bin trace-commons-contributor/\$& --features trace-commons-contributor\/$FEATURE/"
    "--all-features on the FFI dylib build|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& --all-features/"
    "--all-features on a continuation line|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& \\\\\n          --all-features/"
    "Tauri shell enabling it on its dependency|tauri-desktop/src-tauri/Cargo.toml|s#^trace-commons-contributor = \\{ path = \"../../crates/trace-commons-contributor\" \\}#trace-commons-contributor = { path = \"../../crates/trace-commons-contributor\", features = [\"$FEATURE\"] }#m"
    "the feature entering default|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "Insights enabling the forwarded feature by default|crates/trace-commons-contributor-insights/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "platform enabling the feature by default|crates/trace-commons-contributor-platform/Cargo.toml|s/^$FEATURE = .*\$/\$&\\ndefault = [\"$FEATURE\"]/m"
    "--features test-keep-panic-messages on the contributor release build|.github/workflows/release-contributor.yml|s/cargo build --locked --release --bin trace-commons-contributor/\$& --features trace-commons-contributor-ffi\/$PANIC_FEATURE/"
    "--features test-keep-panic-messages on the FFI dylib build|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& --features $PANIC_FEATURE/"
    "test-keep-panic-messages on the staging build|.github/workflows/staging-macos.yml|s/cargo build -p trace-commons-contributor-ffi/\$& --features $PANIC_FEATURE/"
    "the FFI crate enabling it by default|crates/trace-commons-contributor-ffi/Cargo.toml|s/^$PANIC_FEATURE = .*\$/\$&\ndefault = [\"$PANIC_FEATURE\"]/m"
    "the FFI crate enabling it in its normal dependencies|crates/trace-commons-contributor-ffi/Cargo.toml|s/^(anyhow = \"1\")/\$1\ntrace-commons-contributor-ffi = { path = \".\", features = [\"$PANIC_FEATURE\"] }/m"
    "the Tauri shell enabling it on the FFI crate|tauri-desktop/src-tauri/Cargo.toml|s/\A/trace-commons-contributor-ffi = { path = \"..\/..\/crates\/trace-commons-contributor-ffi\", features = [\"$PANIC_FEATURE\"] }\n/"
    "the workspace root enabling it|Cargo.toml|s/\z/\n[workspace.dependencies.seeded]\nfeatures = [\"$PANIC_FEATURE\"]\n/"
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
