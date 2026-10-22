#!/usr/bin/env bash
# Refuse a shipped build that could compile a judgement development feature.
#
# Two features exist only for development, and neither may reach anything
# that ships:
#
#   judgement-development-models  (trace-commons-server) compiles the
#       judgement service's development models into its binary. Those are
#       not on the production allowlist; the development image is a
#       different dstack app serving synthetic or developer-owned data only.
#   judgement-development         (contributor side, added by slice 7) would
#       let a daemon send real sessions to that development service.
#
# The unit tests run under whatever features CI picks and never read a
# release workflow or a Dockerfile, so nothing else stops a later edit from
# enabling one: a `--features` flag on a release line, an `--all-features`
# build, a shipped crate's `default`, a dependency that turns one on, or a
# production image arm that grows a feature. The second name is refused from
# the start, before slice 7 adds it, so no window opens. Design: section 4,
# "A static CI guard" (#1501 review point 7, #1531 review point 5).
#
# Usage:
#   scripts/ci/check-release-judgement-features.sh check [ROOT]
#       Scan the release/packaging files, the shipped crates' manifests, and
#       the judgement image's Dockerfile and workflow. Exit 1 naming each hit.
#   scripts/ci/check-release-judgement-features.sh self-test
#       Copy the scanned files to a scratch tree, prove the clean copy
#       passes, then prove each seeded violation fails.
set -euo pipefail

# Matches both feature names.
FEATURE_PATTERN="judgement-development"
PRODUCTION_FEATURES="near-attestation-collateral"

# Files that build or package a shipped contributor artifact: the same list
# as check-release-credential-store.sh. A missing file is an error, so a
# rename cannot drop one out of the scan. release-*.yml is ALSO globbed.
RELEASE_FILES=(
  .github/workflows/release-apps.yml
  .github/workflows/release-contributor.yml
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

# Crates that ship, the server among them (its binaries are the production
# images). The only permitted mention is the feature's own definition under
# [features].
SHIPPED_MANIFESTS=(
  crates/trace-commons-server/Cargo.toml
  crates/trace-commons-contributor/Cargo.toml
  crates/trace-commons-contributor-sources/Cargo.toml
  crates/trace-commons-contributor-insights/Cargo.toml
  crates/trace-commons-contributor-platform/Cargo.toml
  crates/trace-commons-contributor-ffi/Cargo.toml
  crates/trace-commons-contributor-gtk/Cargo.toml
  tauri-desktop/src-tauri/Cargo.toml
)

# The judgement image. These name the development feature on purpose (the
# development arm), so the rules here are narrower: no --all-features build,
# and a production arm that is exactly the production feature list.
IMAGE_FILES=(
  deploy/judgement/Dockerfile
  .github/workflows/judgement-image.yml
)

# Print each build command in $1 with continuation lines (trailing \ or `)
# joined, so a flag on the next line of the same command is still seen.
joined_commands() {
  awk '{
      line = $0
      if (sub(/[\\`][ \t]*$/, "", line)) { buf = buf line " "; next }
      print buf line; buf = ""
    } END { if (buf != "") print buf }' "$1"
}

all_features_builds() {
  joined_commands "$1" |
    grep -E -- '--all-features' |
    grep -E '(cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?(build|rustc|install|tauri)|tauri[^[:space:]]*[[:space:]]+build)' || true
}

check() {
  local root="${1:-.}" failures=0 file
  cd "$root"

  local release=("${RELEASE_FILES[@]}")
  for file in .github/workflows/release-*.yml; do
    [[ -e "$file" ]] || continue
    [[ " ${release[*]} " == *" $file "* ]] || release+=("$file")
  done
  local images=("${IMAGE_FILES[@]}")
  for file in deploy/*/Dockerfile; do
    [[ -e "$file" ]] || continue
    [[ " ${images[*]} " == *" $file "* ]] || images+=("$file")
  done

  for file in "${release[@]}" "${SHIPPED_MANIFESTS[@]}" "${IMAGE_FILES[@]}"; do
    if [[ ! -f "$file" ]]; then
      echo "MISSING: $file is on the judgement feature scan list but does not exist." >&2
      echo "  Update scripts/ci/check-release-judgement-features.sh if it moved." >&2
      failures=$((failures + 1))
    fi
  done

  # 1. No release/packaging file names either feature, and none builds with
  #    --all-features.
  for file in "${release[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file:$hit names a judgement development feature in a release build path." >&2
      failures=$((failures + 1))
    done < <(grep -n -- "$FEATURE_PATTERN" "$file" || true)
    while IFS= read -r hit; do
      echo "REFUSING: $file: a build command uses --all-features, which enables the judgement development features: $hit" >&2
      failures=$((failures + 1))
    done < <(all_features_builds "$file")
  done

  # 2. Shipped manifests: only the feature's own definition line under
  #    [features] may name it. A `default` list naming it, a dependency
  #    turning it on, or another feature forwarding to it all ship it.
  for file in "${SHIPPED_MANIFESTS[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v pattern="$FEATURE_PATTERN" '
        /^[[:space:]]*\[/ { section = $0; gsub(/[[:space:]]/, "", section) }
        index($0, pattern) == 0 { next }
        /^[[:space:]]*#/ { next }
        {
          line = $0; gsub(/[[:space:]]/, "", line)
          if (section == "[features]" && line ~ /^judgement-development(-models)?=\[\]$/) next
          printf "line %d in %s enables a judgement development feature: %s\n", NR, section, $0
        }' "$file"
    )
  done

  # 3. The images: no --all-features build, and every Dockerfile with a
  #    `production)` profile arm builds exactly the production features.
  #    The judgement Dockerfile must have that arm.
  for file in "${images[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: an image build uses --all-features: $hit" >&2
      failures=$((failures + 1))
    done < <(all_features_builds "$file")
    [[ "$file" == *Dockerfile ]] || continue
    local arms
    arms="$(grep -E '^[[:space:]]*production\)' "$file" || true)"
    if [[ -z "$arms" ]]; then
      if [[ "$file" == deploy/judgement/Dockerfile ]]; then
        echo "REFUSING: $file has no 'production)' profile arm to check." >&2
        failures=$((failures + 1))
      fi
      continue
    fi
    while IFS= read -r arm; do
      if ! grep -Eq "^[[:space:]]*production\)[[:space:]]*features=\"$PRODUCTION_FEATURES\"[[:space:]]*;;[[:space:]]*\\\\?[[:space:]]*\$" <<<"$arm"; then
        echo "REFUSING: $file: the production arm must build exactly \"$PRODUCTION_FEATURES\": $arm" >&2
        failures=$((failures + 1))
      fi
    done <<<"$arms"
  done

  if ((failures > 0)); then
    echo "$failures judgement development feature violation(s)." >&2
    return 1
  fi
  echo "no release, shipped crate or production image can enable a judgement development feature"
}

self_test() {
  local repo scratch status=0
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN

  local file
  for file in "${RELEASE_FILES[@]}" "${SHIPPED_MANIFESTS[@]}" "${IMAGE_FILES[@]}" \
    "$repo"/.github/workflows/release-*.yml "$repo"/deploy/*/Dockerfile; do
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
  # violation. perl rather than sed: BSD sed has no `\n` in a replacement.
  local cases=(
    "the models feature on the contributor release build|.github/workflows/release-contributor.yml|s/cargo build --locked --release --bin trace-commons-contributor/\$& --features judgement-development-models/"
    "the daemon feature on the FFI release build|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& --features trace-commons-contributor\\/judgement-development/"
    "--all-features on a release continuation line|.github/workflows/release-apps.yml|s/cargo build --locked -p trace-commons-contributor-ffi --release/\$& \\\\\n          --all-features/"
    "the models feature entering the server default|crates/trace-commons-server/Cargo.toml|s/^default = \\[\\]/default = [\"judgement-development-models\"]/m"
    "a feature forwarding to the models feature|crates/trace-commons-server/Cargo.toml|s/^judgement-development-models = \\[\\]/judgement-development-models = []\\ndebugging = [\"judgement-development-models\"]/m"
    "the daemon feature entering the contributor default|crates/trace-commons-contributor/Cargo.toml|s/^\\[features\\]\$/[features]\\ndefault = [\"judgement-development\"]/m"
    "the FFI enabling the daemon feature on its dependency|crates/trace-commons-contributor-ffi/Cargo.toml|s/^(trace-commons-contributor = \\{[^}\\n]*)\\}/\$1, features = [\"judgement-development\"] }/m"
    "the production image arm growing the models feature|deploy/judgement/Dockerfile|s/production\\) features=\"near-attestation-collateral\"/production) features=\"near-attestation-collateral judgement-development-models\"/"
    "the production image arm swapped with the development one|deploy/judgement/Dockerfile|s/production\\) features=\"near-attestation-collateral\"/production) features=\"judgement-development-models\"/"
    "the production image arm removed|deploy/judgement/Dockerfile|s/^[ \\t]*production\\).*\\n//m"
    "--all-features in the judgement image build|deploy/judgement/Dockerfile|s/cargo build --locked --release --bin trace-commons-judgement/\$& --all-features/"
    "--all-features in the image workflow|.github/workflows/judgement-image.yml|s/^(\\s*)docker build \\\\\$/\$1cargo build --locked --release --all-features\\n\$&/m"
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
