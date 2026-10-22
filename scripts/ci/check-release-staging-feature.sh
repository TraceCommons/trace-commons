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
# The internal staging build is the one place the feature belongs. It is
# allowed by exact lines, never by pattern: three listed staging files
# (STAGING_FILES) may name it and must not name a production destination;
# the staging workflow runs on workflow_dispatch only and nothing else
# references it; no release workflow's tag filter can match a staging tag;
# and the manifests carry only the exact definitions listed in check 3.
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

# The staging build's own files: the only build, packaging or deploy files
# allowed to name the feature. Listed, so a rename cannot drop one out; each
# must exist, must name the feature and must name no production destination.
STAGING_FILES=(
  .github/workflows/staging-macos.yml         # the staging macOS app, workflow_dispatch only
  deploy/staging-gcp/cloudbuild.yaml          # the staging server binaries
  macos/channels/staging.env                  # the staging bundle channel
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
  crates/trace-commons-attestation/Cargo.toml
  crates/trace-commons-protocol/Cargo.toml
  crates/trace-commons-contributor-gtk/Cargo.toml
  tauri-desktop/src-tauri/Cargo.toml
)

# The staging workflow, the tag prefix its pre-releases push, and the names
# by which another workflow could reach the staging build.
STAGING_WORKFLOW=.github/workflows/staging-macos.yml
STAGING_TAG_PREFIX="staging-"
STAGING_REFERENCES=(staging-macos channels/staging.env)
# The one server bin that may require the feature: the staging pin re-sign
# tool.
STAGING_PINS_BIN="trace-commons-staging-pins"

# Production destinations no staging file may name, matched as fixed,
# case-insensitive strings: the production appcast and update bucket, the
# pilot artifact bucket, the release workflows and the Homebrew tap.
PRODUCTION_DESTINATIONS=(
  storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml
  tracecommons-flatpak
  tc-pilot-artifacts-
  release-apps
  release-contributor
  homebrew-tap
  TraceCommons/tap
)

check() {
  local root="${1:-.}" failures=0 file
  cd "$root"

  local files=("${BUILD_FILES[@]}")
  for file in .github/workflows/release-*.yml; do
    [[ -e "$file" ]] || continue
    [[ " ${files[*]} " == *" $file "* ]] || files+=("$file")
  done

  for file in "${files[@]}" "${SHIPPED_MANIFESTS[@]}" "${STAGING_FILES[@]}"; do
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

  # 3. Manifests of shipped crates. The only permitted mentions are exact
  #    lines, never a pattern:
  #    - under [features]: an empty definition in the server, contributor and
  #      attestation crates; the server and contributor forwarding to the
  #      attestation crate's feature and nowhere else; ingest forwarding to
  #      the server;
  #    - in the server, `required-features = ["near-ai-staging"]` in the one
  #      [[bin]] named trace-commons-staging-pins (the re-sign tool).
  #    Anything else -- a dependency enabling it, `default = [...]`, a
  #    definition forwarding somewhere new or one crate more, the requirement
  #    on another bin -- ships it or widens it.
  for file in "${SHIPPED_MANIFESTS[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v feature="$FEATURE" -v file="$file" -v pins_bin="$STAGING_PINS_BIN" '
        function flush_bin() {
          if (pending != "" && bin_name != pins_bin) printf "%s", pending
          pending = ""; bin_name = ""
        }
        /^[[:space:]]*\[/ { flush_bin(); section = $0; gsub(/[[:space:]]/, "", section) }
        section == "[[bin]]" {
          name = $0; gsub(/[[:space:]]/, "", name)
          if (name ~ /^name=/) { sub(/^name=/, "", name); gsub(/"/, "", name); bin_name = name }
        }
        index($0, feature) == 0 { next }
        /^[[:space:]]*#/ { next }
        {
          line = $0; gsub(/[[:space:]]/, "", line)
          empty = feature "=[]"
          to_attestation = feature "=[\"trace-commons-attestation/" feature "\"]"
          if (section == "[features]") {
            if (file ~ /trace-commons-server\/Cargo.toml$/ && (line == empty || line == to_attestation)) next
            if (file ~ /trace-commons-contributor\/Cargo.toml$/ && (line == empty || line == to_attestation)) next
            if (file ~ /trace-commons-attestation\/Cargo.toml$/ && line == empty) next
            if (file ~ /trace-commons-ingest\/Cargo.toml$/ && line == feature "=[\"trace-commons-server/" feature "\"]") next
          }
          msg = sprintf("line %d in %s enables the staging feature outside its own definition: %s\n", NR, section, $0)
          if (section == "[[bin]]" && file ~ /trace-commons-server\/Cargo.toml$/ && line == "required-features=[\"" feature "\"]") {
            # Permitted only if this [[bin]] turns out to be the re-sign
            # tool; its name may come after this line, so decide at the end
            # of the section.
            pending = pending sprintf("line %d in [[bin]] requires the staging feature on a bin other than %s: %s\n", NR, pins_bin, $0)
            next
          }
          printf "%s", msg
        }
        END { flush_bin() }' "$file"
    )
  done

  # 4. The staging build's own files. Each must name the feature (so a
  #    rename or rewrite cannot quietly drop the staging build out of this
  #    list) and must name no production destination, comments included.
  for file in "${STAGING_FILES[@]}"; do
    [[ -f "$file" ]] || continue
    if ! grep -q -- "$FEATURE" "$file"; then
      echo "REFUSING: $file is a staging file and no longer names '$FEATURE'." >&2
      failures=$((failures + 1))
    fi
    local destination
    for destination in "${PRODUCTION_DESTINATIONS[@]}"; do
      while IFS= read -r hit; do
        echo "REFUSING: $file:$hit: a staging file names the production destination '$destination'." >&2
        failures=$((failures + 1))
      done < <(grep -n -i -F -- "$destination" "$file" || true)
    done
  done

  # 5. The staging workflow runs on a manual dispatch and nothing else: one
  #    top-level `on:` holding a block whose only key is workflow_dispatch
  #    (its inputs may sit beneath). No push, no tag, no workflow_call, no
  #    schedule, no inline list.
  if [[ -f "$STAGING_WORKFLOW" ]]; then
    while IFS= read -r hit; do
      echo "REFUSING: $STAGING_WORKFLOW: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '
        function indent_of(s) { match(s, /^[ ]*/); return RLENGTH }
        /^[[:space:]]*(#|$)/ { next }
        {
          ind = indent_of($0)
          text = $0; sub(/[[:space:]]+#.*$/, "", text); sub(/[[:space:]]+$/, "", text)
        }
        ind == 0 {
          in_on = 0
          if (text ~ /^("on"|'"'"'on'"'"'|on)[[:space:]]*:/) {
            ons++
            value = text; sub(/^[^:]*:[[:space:]]*/, "", value)
            if (value != "") printf "line %d: the trigger must be a block holding only workflow_dispatch, not \"%s\"\n", NR, text
            in_on = 1; child = -1
          }
          next
        }
        in_on {
          if (child < 0) child = ind
          if (ind < child) { printf "line %d: unexpected indentation under on: %s\n", NR, text; next }
          if (ind == child) {
            key = text; sub(/^[[:space:]]*/, "", key)
            if (key != "workflow_dispatch:") printf "line %d: a trigger other than workflow_dispatch: %s\n", NR, key
            else dispatch++
          }
        }
        END {
          if (ons != 1) printf "the workflow must have exactly one top-level on:, found %d\n", ons
          if (dispatch != 1) printf "the workflow must trigger on workflow_dispatch exactly once, found %d\n", dispatch
        }' "$STAGING_WORKFLOW"
    )
  fi

  # 6. No other workflow or action calls the staging workflow or reads the
  #    staging channel file, so neither can be reached from a release.
  local needle
  while IFS= read -r file; do
    [[ "$file" == "$STAGING_WORKFLOW" ]] && continue
    for needle in "${STAGING_REFERENCES[@]}"; do
      while IFS= read -r hit; do
        echo "REFUSING: $file:$hit references '$needle'; only the staging workflow may." >&2
        failures=$((failures + 1))
      done < <(grep -n -F -- "$needle" "$file" || true)
    done
  done < <(find .github/workflows .github/actions -type f 2>/dev/null | sort)

  # 7. The staging pre-release pushes a `staging-` tag. No release workflow
  #    may run on it: each push trigger needs a `tags:` (or `branches:`)
  #    filter, no `tags-ignore:`, and no tag pattern whose literal prefix is
  #    compatible with the staging prefix. Conservative: `st*` or `*` is
  #    refused although it might never meet a staging tag in practice.
  for file in "${files[@]}"; do
    [[ "$file" == .github/workflows/* && -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk -v prefix="$STAGING_TAG_PREFIX" '
        function indent_of(s) { match(s, /^[ ]*/); return RLENGTH }
        function literal_prefix(p,    i, c, out) {
          out = ""
          for (i = 1; i <= length(p); i++) {
            c = substr(p, i, 1)
            if (c == "*" || c == "[" || c == "\\") return out
            # ? and + quantify the character before them.
            if (c == "?" || c == "+") return substr(out, 1, length(out) - 1)
            out = out c
          }
          return out
        }
        function check_pattern(p, n) {
          gsub(/^[[:space:]"'"'"']+|[[:space:]"'"'"']+$/, "", p)
          if (p == "" || substr(p, 1, 1) == "!") return
          lp = literal_prefix(p)
          if (index(prefix, lp) == 1 || index(lp, prefix) == 1)
            printf "line %d: tag pattern \"%s\" can match a %s* staging tag\n", n, p, prefix
        }
        function close_push() {
          if (in_push && !has_tags && !has_branches)
            printf "line %d: a push trigger with no tags: or branches: filter runs on every tag, %s* included\n", push_line, prefix
          in_push = 0; in_tags = 0
        }
        /^[[:space:]]*(#|$)/ { next }
        {
          ind = indent_of($0)
          text = $0; sub(/[[:space:]]+#.*$/, "", text); sub(/[[:space:]]+$/, "", text)
          key = text; sub(/^[[:space:]]*/, "", key)
        }
        ind == 0 {
          close_push(); in_on = 0
          if (text ~ /^("on"|'"'"'on'"'"'|on)[[:space:]]*:/) {
            value = text; sub(/^[^:]*:[[:space:]]*/, "", value)
            if (value ~ /(^|[^a-z_])push([^a-z_]|$)/)
              printf "line %d: an inline push trigger has no tag filter: %s\n", NR, text
            in_on = 1; child = -1
          }
          next
        }
        !in_on { next }
        {
          if (child < 0) child = ind
          if (ind <= child) {
            close_push()
            if (ind == child && key ~ /^push:/) {
              in_push = 1; push_line = NR; has_tags = 0; has_branches = 0; grand = -1
              value = key; sub(/^push:[[:space:]]*/, "", value)
              if (value != "") { printf "line %d: push trigger in a form this check does not read: %s\n", NR, key; in_push = 0 }
            }
            next
          }
        }
        in_push {
          if (grand < 0) grand = ind
          if (ind == grand) {
            in_tags = 0
            if (key ~ /^tags-ignore:/) printf "line %d: tags-ignore: runs on every other tag, %s* included\n", NR, prefix
            else if (key ~ /^branches(-ignore)?:/) has_branches = 1
            else if (key ~ /^tags:/) {
              has_tags = 1; in_tags = 1
              value = key; sub(/^tags:[[:space:]]*/, "", value)
              if (value != "") {
                gsub(/^\[|\]$/, "", value)
                n = split(value, items, ",")
                for (i = 1; i <= n; i++) check_pattern(items[i], NR)
                in_tags = 0
              }
            }
            next
          }
          if (in_tags && ind > grand && key ~ /^-/) { item = key; sub(/^-[[:space:]]*/, "", item); check_pattern(item, NR) }
        }
        END { close_push() }' "$file"
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
  for file in "${BUILD_FILES[@]}" "${SHIPPED_MANIFESTS[@]}" "${STAGING_FILES[@]}" \
    "$repo"/.github/workflows/*.yml "$repo"/.github/workflows/*.yaml; do
    [[ "$file" == *'*'* ]] && continue
    file="${file#"$repo"/}"
    mkdir -p "$scratch/$(dirname "$file")"
    cp "$repo/$file" "$scratch/$file"
  done

  if ! (check "$scratch" >/dev/null 2>&1); then
    echo "self-test: the unmodified tree should pass and did not" >&2
    (check "$scratch") || true
    return 1
  fi

  # Each accepted case seeds one line the check must still permit: the exact
  # staging allowances that the tree does not carry yet. Without these, a
  # refusal case below could pass with its allowance missing altogether.
  local accepted=(
    "the contributor forwarding to the attestation crate|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\"]/m"
    "the server forwarding to the attestation crate|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\"]/m"
    "the attestation crate's own empty definition|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\n/m"
    "the re-sign tool requiring it|crates/trace-commons-server/Cargo.toml|s/^\\[dev-dependencies\\]/[[bin]]\\nname = \"trace-commons-staging-pins\"\\npath = \"src\/bin\/trace-commons-staging-pins.rs\"\\nrequired-features = [\"$FEATURE\"]\\n\\n\$&/m"
    "the staging workflow taking a dispatch input|.github/workflows/staging-macos.yml|s/^  workflow_dispatch:\\n/\$&    inputs:\\n      ref:\\n        type: string\\n        required: false\\n/m"
  )

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
    # The staging allowances: each case is one step past one of them.
    "the staging workflow no longer naming it|.github/workflows/staging-macos.yml|s/$FEATURE/near-ai-other/g"
    "the staging Cloud Build file no longer naming it|deploy/staging-gcp/cloudbuild.yaml|s/$FEATURE/near-ai-other/g"
    "the staging channel file no longer naming it|macos/channels/staging.env|s/$FEATURE/near-ai-other/g"
    "the staging channel file naming the production appcast|macos/channels/staging.env|s/\\z/TC_FEED=https:\\/\\/storage.googleapis.com\\/tracecommons-flatpak\\/updates\\/appcast.xml\\n/"
    "the staging channel file naming the production update bucket|macos/channels/staging.env|s/\\z/TC_BUCKET=tracecommons-flatpak\\n/"
    "the staging Cloud Build file naming the pilot artifact bucket|deploy/staging-gcp/cloudbuild.yaml|s/\\z/substitutions:\\n  _BUCKET: 'tc-pilot-artifacts-20260518'\\n/"
    "the staging workflow naming the Homebrew tap|.github/workflows/staging-macos.yml|s/\\z/      - run: git clone https:\\/\\/github.com\\/TraceCommons\\/homebrew-tap.git\\n/"
    "the staging workflow naming the cask tap|.github/workflows/staging-macos.yml|s/\\z/      - run: brew tap TraceCommons\\/tap\\n/"
    "the staging workflow naming release-apps|.github/workflows/staging-macos.yml|s/\\z/      - uses: .\\/.github\\/workflows\\/release-apps.yml\\n/"
    "the staging workflow naming release-contributor|.github/workflows/staging-macos.yml|s/\\z/      - uses: .\\/.github\\/workflows\\/release-contributor.yml\\n/"
    "push: added to the staging workflow|.github/workflows/staging-macos.yml|s/^on:\\n/on:\\n  push:\\n/m"
    "workflow_call: added to the staging workflow|.github/workflows/staging-macos.yml|s/^on:\\n/on:\\n  workflow_call:\\n/m"
    "a trigger added after the dispatch in the staging workflow|.github/workflows/staging-macos.yml|s/^  workflow_dispatch:\\n/\$&  schedule:\\n    - cron: \"0 0 * * *\"\\n/m"
    "an inline trigger list on the staging workflow|.github/workflows/staging-macos.yml|s/^on:\\n  workflow_dispatch:\\n/on: [workflow_dispatch, push]\\n/m"
    "a second on: key in the staging workflow|.github/workflows/staging-macos.yml|s/\\z/\"on\": push\\n/"
    "a release workflow calling the staging workflow|.github/workflows/release-apps.yml|s/\\z/  staging:\\n    uses: .\\/.github\\/workflows\\/staging-macos.yml\\n/"
    "a release workflow reading the staging channel file|.github/workflows/release-contributor.yml|s/\\A/# source macos\\/channels\\/staging.env\\n/"
    "another workflow calling the staging workflow|.github/workflows/ci.yml|s/\\A/# .github\\/workflows\\/staging-macos.yml\\n/"
    "a release tag filter matching the staging tag prefix|.github/workflows/release-apps.yml|s/\"app-v\\*\"/\"staging-*\"/"
    "a release tag filter matching every tag|.github/workflows/release-contributor.yml|s/(      - \"contributor-v\\*\"\\n)/\$1      - \"*\"\\n/"
    "a release tag filter matching by a short prefix|.github/workflows/release-apps.yml|s/\"app-v\\*\"/\"st*\"/"
    "a release tag filter in inline form|.github/workflows/release-apps.yml|s/    tags:\\n      - \"app-v\\*\"\\n/    tags: [\"app-v*\", \"staging-*\"]\\n/"
    "a release push trigger with no tag filter|.github/workflows/release-contributor.yml|s/    tags:\\n      - \"contributor-v\\*\"\\n//"
    "a release push trigger ignoring tags instead|.github/workflows/release-apps.yml|s/    tags:\\n      - \"app-v\\*\"\\n/    tags-ignore:\\n      - \"app-v*\"\\n/"
    "a release workflow with an inline push trigger|.github/workflows/release-contributor.yml|s/^on:\\n  push:\\n    tags:\\n      - \"contributor-v\\*\"\\n/on: [push, workflow_dispatch]\\nx:\\n/m"
    "the contributor forward widened by one more crate|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\", \"trace-commons-protocol\/$FEATURE\"]/m"
    "the server forward widened by one more crate|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\", \"trace-commons-gate-enclave\/near-ai-scorer\"]/m"
    "the contributor forwarding somewhere other than attestation|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\\]\$/$FEATURE = [\"trace-commons-protocol\/$FEATURE\"]/m"
    "the attestation definition forwarding anywhere|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = [\"collateral-client\"]\\n/m"
    "the attestation feature entering its default|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\ndefault = [\"$FEATURE\"]\\n/m"
    "the protocol crate defining it|crates/trace-commons-protocol/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\n/m"
    "the re-sign tool's requirement on another bin|crates/trace-commons-server/Cargo.toml|s/(name = \"trace-commons-witness\"\\npath = \"[^\"]*\"\\n)/\$1required-features = [\"$FEATURE\"]\\n/"
    "the re-sign tool's requirement on a near-named bin|crates/trace-commons-server/Cargo.toml|s/^\\[dev-dependencies\\]/[[bin]]\\nname = \"trace-commons-staging-pins-extra\"\\npath = \"src\/bin\/x.rs\"\\nrequired-features = [\"$FEATURE\"]\\n\\n\$&/m"
    "the re-sign tool requiring it with another feature|crates/trace-commons-server/Cargo.toml|s/^\\[dev-dependencies\\]/[[bin]]\\nname = \"trace-commons-staging-pins\"\\npath = \"src\/bin\/trace-commons-staging-pins.rs\"\\nrequired-features = [\"$FEATURE\", \"gcs-client\"]\\n\\n\$&/m"
    "the re-sign tool's requirement in the ingest manifest|crates/trace-commons-ingest/Cargo.toml|s/\\z/\\n[[bin]]\\nname = \"trace-commons-staging-pins\"\\npath = \"src\/bin\/x.rs\"\\nrequired-features = [\"$FEATURE\"]\\n/"
  )
  local entry desc target program
  for entry in "${accepted[@]}"; do
    IFS='|' read -r desc target program <<<"$entry"
    cp "$scratch/$target" "$scratch/.before"
    perl -0pi -e "$program" "$scratch/$target"
    if cmp -s "$scratch/$target" "$scratch/.before"; then
      echo "self-test: seeding '$desc' changed nothing in $target; the fixture drifted" >&2
      status=1
    elif ! (check "$scratch" >/dev/null 2>&1); then
      echo "self-test: '$desc' is an allowed line and was refused" >&2
      (check "$scratch" 2>&1 | sed 's/^/  /') || true
      status=1
    else
      echo "self-test: accepted as expected: $desc"
    fi
    mv "$scratch/.before" "$scratch/$target"
  done
  for entry in "${cases[@]}"; do
    IFS='|' read -r desc target program <<<"$entry"
    cp "$scratch/$target" "$scratch/.before"
    perl -0pi -e "$program" "$scratch/$target"
    if cmp -s "$scratch/$target" "$scratch/.before"; then
      echo "self-test: seeding '$desc' changed nothing in $target; the fixture drifted" >&2
      status=1
    elif (check "$scratch" >/dev/null 2>&1); then
      echo "self-test: '$desc' was NOT refused" >&2
      status=1
    else
      echo "self-test: refused as expected: $desc"
    fi
    mv "$scratch/.before" "$scratch/$target"
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
