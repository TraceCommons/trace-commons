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
# The macOS bundle scripts pick their channel from macos/channels/
# $TC_CHANNEL.env (spec 4.1), so the feature name never appears in them and
# check 1 cannot see a release build switched to staging. Checks 6 and 8
# close that: no release, packaging or deploy file may reference the staging
# channel file, and none may set or default TC_CHANNEL to anything but the
# literal `production`. Reading it (`$TC_CHANNEL`, `${TC_CHANNEL}`) is fine.
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
  macos/scripts/info-plist.sh                 # reads the bundle channel (spec 4.1)
  macos/scripts/channel.sh                    # loads the bundle channel file
  macos/channels/production.env               # the production bundle channel
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
STAGING_REFERENCES=(staging-macos staging.env)
# The one server bin that may require the feature: the staging pin re-sign
# tool.
STAGING_PINS_BIN="trace-commons-staging-pins"

# Production destinations no staging file may name, matched as fixed,
# case-insensitive strings: the production appcast and update bucket, the
# pilot artifact bucket, the release workflows and the Homebrew tap.
PRODUCTION_DESTINATIONS=(
  # Redundant with tracecommons-flatpak below; kept so the list names the
  # production appcast spec 6.2 refuses by itself.
  storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml
  tracecommons-flatpak
  tc-pilot-artifacts-
  release-apps
  release-contributor
  homebrew-tap
  TraceCommons/tap
)

# Production credentials no staging file may name. These are repository
# secrets, readable from any workflow, so a staging run naming one could
# publish to production with the destination held in a variable. Matched as
# whole names: a staging secret of its own (STAGING_..., ..._STAGING) passes.
# The only secrets the staging workflow may read: the Developer ID and
# notary credentials it shares with production, and its own staging-named
# ones. An allow-list, so a production secret added later is refused without
# anyone remembering to list it above. The job token is `github.token`.
STAGING_WORKFLOW_SECRETS=(
  MACOS_CERTIFICATE_P12_BASE64
  MACOS_CERTIFICATE_PASSWORD
  MACOS_SIGNING_IDENTITY
  MACOS_NOTARY_ASC_KEY_P8_BASE64
  MACOS_NOTARY_ASC_KEY_ID
  MACOS_NOTARY_ASC_ISSUER_ID
  TC_STAGING_SPARKLE_PUBLIC_ED_KEY
  TC_STAGING_SPARKLE_PRIVATE_ED_KEY
  TC_STAGING_GCP_WIF_PROVIDER
  TC_STAGING_GCP_PUBLISHER_SA
  GITHUB_TOKEN
)

PRODUCTION_SECRETS=(
  HOMEBREW_TAP_TOKEN
  GCP_FLATPAK_PUBLISHER_SA
  GCP_WIF_PROVIDER
  WINGET_PKGS_TOKEN
  SPARKLE_PUBLIC_ED_KEY
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

  # 3b. No TOML escape in a shipped manifest. `"near-ai-\u0073taging"`
  #     decodes to the feature name and never contains it, so the scans
  #     above cannot see it. No shipped manifest needs a backslash today.
  for file in "${SHIPPED_MANIFESTS[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file:$hit: a backslash escape in a shipped manifest can spell '$FEATURE' past this check." >&2
      failures=$((failures + 1))
    done < <(grep -n -F -- "\\" "$file" | grep -v -E '^[0-9]+:[[:space:]]*#' || true)
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
    local secret
    for secret in "${PRODUCTION_SECRETS[@]}"; do
      while IFS= read -r hit; do
        echo "REFUSING: $file:$hit: a staging file names the production credential '$secret'." >&2
        failures=$((failures + 1))
      done < <(grep -n -E -- "(^|[^A-Za-z0-9_])$secret([^A-Za-z0-9_]|\$)" "$file" || true)
    done
    # A GitHub release from a staging file is a pre-release, never a full
    # one. Continuation lines are joined first, as in check 2.
    while IFS= read -r hit; do
      echo "REFUSING: $file: a staging release must be a pre-release: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '{
          line = $0
          if (sub(/[\\`][ \t]*$/, "", line)) { buf = buf line " "; next }
          print buf line; buf = ""
        } END { if (buf != "") print buf }' "$file" |
        grep -E 'gh[[:space:]]+release[[:space:]]+(create|edit)' |
        grep -v -E -- '--prerelease([[:space:]]|$)' || true
    )
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

  # 5b. Any job environment in the staging workflow is exactly `staging`,
  #     inline or as a block's name:. A job may name no environment at all
  #     (staging design S14, #1561: no approval gate; the staging secrets are
  #     repository secrets with staging names, which check 4 holds). The
  #     release environment carries the production publishing identity (its
  #     WIF subject is bound to it).
  if [[ -f "$STAGING_WORKFLOW" ]]; then
    while IFS= read -r hit; do
      echo "REFUSING: $STAGING_WORKFLOW: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '
        function indent_of(s) { match(s, /^[ ]*/); return RLENGTH }
        function unquote(v) { gsub(/^["'"'"']|["'"'"']$/, "", v); return v }
        function close_env() {
          if (in_env && !named) printf "line %d: an environment block with no name: staging\n", env_line
          in_env = 0
        }
        /^[[:space:]]*(#|$)/ { next }
        {
          ind = indent_of($0)
          text = $0; sub(/[[:space:]]+#.*$/, "", text); sub(/[[:space:]]+$/, "", text)
          key = text; sub(/^[[:space:]]*/, "", key)
        }
        in_env && ind <= env_ind { close_env() }
        in_env {
          if (key ~ /^name:/) {
            value = key; sub(/^name:[[:space:]]*/, "", value)
            named = 1
            if (unquote(value) != "staging") printf "line %d: the environment must be staging, not %s\n", NR, value
          }
          next
        }
        key ~ /^environment:/ {
          value = key; sub(/^environment:[[:space:]]*/, "", value)
          if (value == "") { in_env = 1; named = 0; env_ind = ind; env_line = NR }
          else if (unquote(value) != "staging") printf "line %d: the environment must be staging, not %s\n", NR, value
        }
        END { close_env() }' "$STAGING_WORKFLOW"
    )
  fi

  # 5c. The staging workflow reads only the secrets on STAGING_WORKFLOW_SECRETS,
  #     each by name. Any other use of `secrets` inside an expression (an
  #     index, toJSON(secrets), an unlisted name) is refused, and so is an
  #     expression that does not close on its own line, which this scan
  #     could not read.
  if [[ -f "$STAGING_WORKFLOW" ]]; then
    while IFS= read -r hit; do
      echo "REFUSING: $STAGING_WORKFLOW: $hit" >&2
      failures=$((failures + 1))
    done < <(
      ALLOWED="${STAGING_WORKFLOW_SECRETS[*]}" perl -ne '
        BEGIN { %ok = map { $_ => 1 } split / /, $ENV{ALLOWED} }
        next if /^\s*#/;
        my $line = $_;
        while ($line =~ /\$\{\{(.*?)\}\}/g) {
          my $expr = $1;
          while ($expr =~ /(?<![A-Za-z0-9_.])secrets\b(\s*\.\s*([A-Za-z_][A-Za-z0-9_]*))?/g) {
            if (!defined $2) { print "line $.: secrets read other than by name\n" }
            elsif (!$ok{$2}) { print "line $.: the secret $2 is not on the staging list\n" }
          }
        }
        (my $rest = $line) =~ s/\$\{\{.*?\}\}//g;
        print "line $.: an expression that does not close on its line\n" if $rest =~ /\$\{\{/;
      ' "$STAGING_WORKFLOW"
    )
  fi

  # 6. No other workflow or action, and no release, packaging or deploy file,
  #    calls the staging workflow or reads the staging channel file, so
  #    neither can be reached from a release.
  local needle reference_files=()
  while IFS= read -r file; do
    reference_files+=("$file")
  done < <(find .github/workflows .github/actions -type f 2>/dev/null | sort)
  for file in "${files[@]}"; do
    [[ " ${reference_files[*]} " == *" $file "* ]] || reference_files+=("$file")
  done
  for file in "${reference_files[@]}"; do
    [[ "$file" == "$STAGING_WORKFLOW" || ! -f "$file" ]] && continue
    for needle in "${STAGING_REFERENCES[@]}"; do
      while IFS= read -r hit; do
        echo "REFUSING: $file:$hit references '$needle'; only the staging workflow may." >&2
        failures=$((failures + 1))
      done < <(grep -n -F -- "$needle" "$file" || true)
    done
  done

  # 7. The staging pre-release pushes a `staging-` tag. No release workflow
  #    may run on it: each push trigger needs a `tags:` (or `branches:`)
  #    filter, no `tags-ignore:`, and no tag pattern whose literal prefix is
  #    compatible with the staging prefix. Conservative: `st*` or `*` is
  #    refused although it might never meet a staging tag in practice.
  #    Publishing that pre-release also fires `release` and `create`, and
  #    `workflow_run` chains off any workflow, so none of the three may
  #    trigger a release workflow.
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
            if (value ~ /(^|[^a-z_])(release|create|workflow_run)([^a-z_]|$)/)
              printf "line %d: an inline trigger the staging pre-release also fires: %s\n", NR, text
            in_on = 1; child = -1
          }
          next
        }
        !in_on { next }
        {
          if (child < 0) child = ind
          if (ind <= child) {
            close_push()
            if (ind == child && key ~ /^["'"'"']?(release|create|workflow_run)["'"'"']?[[:space:]]*:/)
              printf "line %d: the %s trigger also fires on the staging pre-release or its tag\n", NR, key
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

  # 8. No release, packaging or deploy file sets or defaults TC_CHANNEL to
  #    anything but the literal `production` (spec 4.1: the channel file
  #    carries the feature, so a staging channel is a staging build). A read
  #    -- $TC_CHANNEL, ${TC_CHANNEL} -- is fine; an assignment (sh, YAML
  #    env:, PowerShell $env:) or a ${TC_CHANNEL<op>value} expansion must
  #    name production. Comment lines are skipped.
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r hit; do
      echo "REFUSING: $file: $hit" >&2
      failures=$((failures + 1))
    done < <(
      awk '
        function unquote(v) { gsub(/^["'"'"']|["'"'"';]$/, "", v); gsub(/["'"'"']$/, "", v); return v }
        /^[[:space:]]*#/ { next }
        {
          rest = $0
          while ((i = index(rest, "TC_CHANNEL")) > 0) {
            before = substr(rest, 1, i - 1); after = substr(rest, i + 10); rest = after
            if (after ~ /^[A-Za-z0-9_]/ || before ~ /[A-Za-z0-9_]$/) continue
            if (before ~ /\$\{$/) {
              if (after ~ /^\}/) continue
              val = after; sub(/\}.*$/, "", val)
              if (val !~ /^:?[-=]production$/)
                printf "line %d: TC_CHANNEL expanded with a value other than production: %s\n", NR, $0
              continue
            }
            if (before ~ /\$$/) continue
            if (match(after, /^["'"'"']?[[:space:]]*(\+?=|:)/) && after !~ /^["'"'"']?[[:space:]]*==/) {
              val = substr(after, RLENGTH + 1)
              sub(/^[[:space:]]+/, "", val); sub(/[[:space:]]+#.*$/, "", val); sub(/[[:space:]]+$/, "", val)
              val = unquote(val)
              if (val != "production" && val !~ /^\$\{TC_CHANNEL:?[-=]production\}$/)
                printf "line %d: TC_CHANNEL set to something other than production: %s\n", NR, $0
            }
          }
        }' "$file"
    )
  done

  # 9. The bundle's Info.plist, rendered with no channel chosen, declares
  #    exactly one TCChannel and it is `production`. The app refuses to start
  #    when TCChannel is missing or disagrees with its dylib (spec 4.3), so a
  #    plist that dropped the key would ship a bundle that never starts, and
  #    one naming staging would refuse around a production dylib. When
  #    info-plist.sh reads a channel file, that file must join the self-test
  #    copy below or the self-test's clean tree stops rendering.
  local plist channels
  if plist="$(env -u TC_CHANNEL -u TC_SPARKLE_PUBLIC_ED_KEY bash macos/scripts/info-plist.sh 0.0.0 0 2>/dev/null)"; then
    channels="$(grep -c '<key>TCChannel</key>' <<<"$plist" || true)"
    if [[ "$channels" != 1 ]] \
      || ! grep -q '<key>TCChannel</key><string>production</string>' <<<"$plist"; then
      echo "REFUSING: macos/scripts/info-plist.sh: the rendered plist must declare TCChannel exactly once, as production (found $channels)" >&2
      failures=$((failures + 1))
    fi
  else
    echo "REFUSING: macos/scripts/info-plist.sh did not render" >&2
    failures=$((failures + 1))
  fi

  # 10. Both macOS build scripts check the bundle's channel before it can be
  #     signed, and the check reads the FFI library as well as the plist and
  #     the CLI (spec 4.3). Only CI runs this file on every PR; the check
  #     itself runs only inside a real bundle build, so without this a
  #     deleted call would go unnoticed until a DMG that cannot start ships.
  for file in macos/scripts/make-app-bundle.sh macos/scripts/make-release-dmg.sh; do
    [[ -f "$file" ]] || continue
    if ! grep -q -x -F -- 'assert_bundle_channel "$APP"' "$file"; then
      echo "REFUSING: $file no longer checks the bundle channel (assert_bundle_channel \"\$APP\")" >&2
      failures=$((failures + 1))
    fi
  done
  if [[ -f macos/scripts/channel.sh ]] \
    && ! grep -q -x -F -- '  lib_report="$(library_channel "$app/Contents/Frameworks/libtrace_commons_contributor_ffi.dylib")" || {' macos/scripts/channel.sh; then
    echo "REFUSING: macos/scripts/channel.sh: the bundle channel check no longer reads the FFI library" >&2
    failures=$((failures + 1))
  fi

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
    "$repo"/.github/workflows/*.yml "$repo"/.github/workflows/*.yaml \
    "$repo"/.github/actions/*/action.yml; do
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
    "the server's own empty definition|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = []/m"
    "the contributor's own empty definition|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = []/m"
    "the attestation crate's own empty definition|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\n/m"
    "the staging workflow taking a dispatch input|.github/workflows/staging-macos.yml|s/^  workflow_dispatch:\\n/\$&    inputs:\\n      ref:\\n        type: string\\n        required: false\\n/m"
    "a build script defaulting the channel to production and reading its file|macos/scripts/make-app-bundle.sh|s/\\z/TC_CHANNEL=\"\\\${TC_CHANNEL:-production}\"\\nsource \"\\\$(dirname \"\\\$0\")\\/..\\/channels\\/\\\${TC_CHANNEL}.env\"\\nif [[ \"\\\$TC_CHANNEL\" == production ]]; then :; fi\\n/"
    "a release workflow pinning the production channel|.github/workflows/release-apps.yml|s/\\z/env:\\n  TC_CHANNEL: production\\n/"
    "the staging macOS job declaring the staging environment|.github/workflows/staging-macos.yml|s/^    runs-on: macos-26\\n/\$&    environment: staging\\n/m"
    "the staging workflow declaring the staging environment|.github/workflows/staging-macos.yml|s/^    runs-on: ubuntu-latest\\n/\$&    environment:\\n      name: staging\\n/m"
    "the staging workflow reading its own staging secrets|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.TC_STAGING_SPARKLE_PUBLIC_ED_KEY }} \\\${{ secrets.TC_STAGING_GCP_WIF_PROVIDER }}\"\\n/"
    "the staging workflow reading the job token as a secret|.github/workflows/staging-macos.yml|s/\\z/      - run: gh api user\\n        env:\\n          GH_TOKEN: \\\${{ secrets.GITHUB_TOKEN }}\\n/"
    "the staging workflow creating a pre-release over two lines|.github/workflows/staging-macos.yml|s/\\z/      - run: |\\n          gh release create staging-x dist\\/a.dmg \\\\\\n            --prerelease\\n/"
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
    "the server definition forwarding somewhere new|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = [\"trace-commons-gate-enclave\\/near-ai-scorer\"]/m"
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
    "the contributor forward widened by one more crate|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\", \"trace-commons-protocol\/$FEATURE\"]/m"
    "the server forward widened by one more crate|crates/trace-commons-server/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = [\"trace-commons-attestation\/$FEATURE\", \"trace-commons-gate-enclave\/near-ai-scorer\"]/m"
    "the contributor forwarding somewhere other than attestation|crates/trace-commons-contributor/Cargo.toml|s/^$FEATURE = \\[\"trace-commons-attestation\/$FEATURE\"\\]\$/$FEATURE = [\"trace-commons-protocol\/$FEATURE\"]/m"
    "the attestation definition forwarding anywhere|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = [\"collateral-client\"]\\n/m"
    "the attestation feature entering its default|crates/trace-commons-attestation/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\ndefault = [\"$FEATURE\"]\\n/m"
    "the protocol crate defining it|crates/trace-commons-protocol/Cargo.toml|s/^\\[features\\]\\n/\$&$FEATURE = []\\n/m"
    "the re-sign tool's requirement on another bin|crates/trace-commons-server/Cargo.toml|s/(name = \"trace-commons-witness\"\\npath = \"[^\"]*\"\\n)/\$1required-features = [\"$FEATURE\"]\\n/"
    "the re-sign tool's requirement on a near-named bin|crates/trace-commons-server/Cargo.toml|s/^\\[dev-dependencies\\]/[[bin]]\\nname = \"trace-commons-staging-pins-extra\"\\npath = \"src\/bin\/x.rs\"\\nrequired-features = [\"$FEATURE\"]\\n\\n\$&/m"
    "the re-sign tool requiring it with another feature|crates/trace-commons-server/Cargo.toml|s/^\\[dev-dependencies\\]/[[bin]]\\nname = \"trace-commons-staging-pins\"\\npath = \"src\/bin\/trace-commons-staging-pins.rs\"\\nrequired-features = [\"$FEATURE\", \"gcs-client\"]\\n\\n\$&/m"
    "the re-sign tool's requirement in the ingest manifest|crates/trace-commons-ingest/Cargo.toml|s/\\z/\\n[[bin]]\\nname = \"trace-commons-staging-pins\"\\npath = \"src\/bin\/x.rs\"\\nrequired-features = [\"$FEATURE\"]\\n/"
    # A staging file deleted outright: only the existence check catches it.
    "the staging channel file deleted|macos/channels/staging.env|DELETE"
    "the staging workflow with a second, empty on: block|.github/workflows/staging-macos.yml|s/\\z/on:\\n/"
    "the staging workflow with an empty on: block|.github/workflows/staging-macos.yml|s/^on:\\n  workflow_dispatch:\\n/on:\\n/m"
    "a shallower sibling key under the staging workflow's on:|.github/workflows/staging-macos.yml|s/^on:\\n  workflow_dispatch:\\n/on:\\n    workflow_dispatch:\\n  push:\\n/m"
    "a composite action reading the staging channel file|.github/actions/trim-cargo-cache/action.yml|s/\\A/# source macos\\/channels\\/staging.env\\n/"
    "the staging channel file naming a production bucket in upper case|macos/channels/staging.env|s/\\z/TC_BUCKET=TRACECOMMONS-FLATPAK\\n/"
    "a release tag filter longer than the staging prefix|.github/workflows/release-apps.yml|s/\"app-v\\*\"/\"staging-2026*\"/"
    "a release push trigger with tags-ignore: beside its tags:|.github/workflows/release-apps.yml|s/    tags:\\n      - \"app-v\\*\"\\n/\$&    tags-ignore:\\n      - \"x\"\\n/"
    # The channel indirection (spec 4.1): a build script reads
    # channels/\$TC_CHANNEL.env, so the feature name never appears in it.
    "a build script reading the staging channel file|macos/scripts/make-app-bundle.sh|s/\\z/source \"\\\$(dirname \"\\\$0\")\\/..\\/channels\\/staging.env\"\\n/"
    "a build script reading staging.env by a relative name|macos/scripts/make-release-dmg.sh|s/\\z/cd channels \\&\\& . .\\/staging.env\\n/"
    "a release workflow setting the staging channel|.github/workflows/release-apps.yml|s/\\z/env:\\n  TC_CHANNEL: staging\\n/"
    "a release workflow taking the channel from an input|.github/workflows/release-contributor.yml|s/\\z/env:\\n  TC_CHANNEL: \\\${{ inputs.channel }}\\n/"
    "a build script defaulting the channel to staging|macos/scripts/make-release-dmg.sh|s/\\z/TC_CHANNEL=\"\\\${TC_CHANNEL:-staging}\"\\n/"
    "a build script defaulting the channel inline to staging|macos/scripts/make-app-bundle.sh|s/\\z/source \"channels\\/\\\${TC_CHANNEL:-staging}.env\"\\n/"
    "a build script exporting the channel from its arguments|macos/scripts/make-app-bundle.sh|s/\\z/export TC_CHANNEL=\\\$1\\n/"
    "a PowerShell build script setting the staging channel|windows/scripts/make-msix.ps1|s/\\z/\\\$env:TC_CHANNEL = 'staging'\\n/"
    # Release workflows run on events the staging pre-release also fires.
    "a release workflow running on a published release|.github/workflows/release-apps.yml|s/^on:\\n/on:\\n  release:\\n    types: [published]\\n/m"
    "a release workflow running on a created tag|.github/workflows/release-apps.yml|s/^on:\\n/on:\\n  create:\\n/m"
    "a release workflow running after another workflow|.github/workflows/release-contributor.yml|s/^on:\\n/on:\\n  workflow_run:\\n    workflows: [staging-build]\\n    types: [completed]\\n/m"
    "a release workflow with an inline release trigger|.github/workflows/release-contributor.yml|s/^on:\\n  push:\\n    tags:\\n      - \"contributor-v\\*\"\\n/on: release\\nx:\\n/m"
    # Production credentials the staging build must not borrow.
    "the staging workflow using the release environment|.github/workflows/staging-macos.yml|s/^    runs-on: ubuntu-latest\\n/\$&    environment: release\\n/m"
    "the staging macOS job using the release environment|.github/workflows/staging-macos.yml|s/^    runs-on: macos-26\\n/\$&    environment: release\\n/m"
    "the staging macOS job using an environment of another name|.github/workflows/staging-macos.yml|s/^    runs-on: macos-26\\n/\$&    environment:\\n      name: production\\n/m"
    "the staging workflow using the release environment by name|.github/workflows/staging-macos.yml|s/^    runs-on: ubuntu-latest\\n/\$&    environment:\\n      name: 'release'\\n/m"
    "the staging workflow reading a secret not on its list|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.AZURE_CLIENT_ID }}\"\\n/"
    "the staging workflow reading an unlisted secret with a staging name|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.STAGING_SPARKLE_PUBLIC_ED_KEY }}\"\\n/"
    "the staging workflow reading an unlisted secret by index|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets['AZURE_CLIENT_ID'] }}\"\\n/"
    "the staging workflow passing every secret on|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ toJSON(secrets) }}\"\\n/"
    "the staging workflow reading the tap token|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.HOMEBREW_TAP_TOKEN }}\"\\n/"
    "the staging workflow reading the flatpak publisher|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.GCP_FLATPAK_PUBLISHER_SA }}\"\\n/"
    "the staging workflow reading the winget token|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets['WINGET_PKGS_TOKEN'] }}\"\\n/"
    "the staging workflow reading the production Sparkle key|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.SPARKLE_PUBLIC_ED_KEY }}\"\\n/"
    "the staging workflow reading the production WIF provider|.github/workflows/staging-macos.yml|s/\\z/      - run: echo \"\\\${{ secrets.GCP_WIF_PROVIDER }}\"\\n/"
    "the staging workflow creating a full release|.github/workflows/staging-macos.yml|s/\\z/      - run: gh release create staging-x dist\\/a.dmg\\n/"
    # The bundle's declared channel (spec 4.3).
    "the bundle plist without TCChannel|macos/scripts/info-plist.sh|s/^ *<key>TCChannel<\\/key><string>\\\$\\{TC_CHANNEL\\}<\\/string>\\n//m"
    "the bundle plist naming the staging channel|macos/scripts/info-plist.sh|s/<key>TCChannel<\\/key><string>\\\$\\{TC_CHANNEL\\}</<key>TCChannel<\\/key><string>staging</"
    "the bundle plist declaring TCChannel twice|macos/scripts/info-plist.sh|s/^ *<key>TCChannel<\\/key><string>\\\$\\{TC_CHANNEL\\}<\\/string>\\n/\$&\$&/m"
    # The bundle channel files (spec 4.1): production's file is a release
    # file, and the loader's default is production.
    "the production channel file giving the dylib the feature|macos/channels/production.env|s/^TC_FFI_CARGO_FEATURES=\\n/TC_FFI_CARGO_FEATURES=trace-commons-contributor\\/near-ai-staging\\n/m"
    "the production channel file giving the CLI the feature|macos/channels/production.env|s/^TC_CLI_CARGO_FEATURES=\\n/TC_CLI_CARGO_FEATURES=near-ai-staging\\n/m"
    "the channel loader defaulting to staging|macos/scripts/channel.sh|s/TC_CHANNEL:-production/TC_CHANNEL:-staging/"
    # The bundle channel check before signing (spec 4.3).
    "the release DMG script no longer checking the bundle channel|macos/scripts/make-release-dmg.sh|s/^assert_bundle_channel \"\\\$APP\"\n//m"
    "the release DMG script checking the bundle channel only in a comment|macos/scripts/make-release-dmg.sh|s/^assert_bundle_channel \"\\\$APP\"\n/# assert_bundle_channel \"\\\$APP\"\n/m"
    "the bundle script no longer checking the bundle channel|macos/scripts/make-app-bundle.sh|s/^assert_bundle_channel \"\\\$APP\"\n//m"
    "the bundle channel check no longer reading the FFI library|macos/scripts/channel.sh|s/^  lib_report=.*\n/  lib_report=\"\\\$TC_CHANNEL \\\$TC_URL_SCHEME\"\n/m"
    # A TOML escape spells the feature without its literal name.
    "the FFI enabling it through a TOML escape|crates/trace-commons-contributor-ffi/Cargo.toml|s#^(trace-commons-contributor = \\{[^}\\n]*)\\}#\$1, features = [\"near-ai-\\\\u0073taging\"] }#m"
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
    # DELETE removes the file instead of editing it.
    if [[ "$program" == DELETE ]]; then
      rm "$scratch/$target"
    else
      perl -0pi -e "$program" "$scratch/$target"
    fi
    if [[ -f "$scratch/$target" ]] && cmp -s "$scratch/$target" "$scratch/.before"; then
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
