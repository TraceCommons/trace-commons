#!/usr/bin/env bash
# The bundle channel: read macos/channels/<channel>.env (staging design 4.1).
#
# Sourced by info-plist.sh, make-app-bundle.sh and make-release-dmg.sh. Each
# calls `load_channel`, which reads TC_CHANNEL (unset or empty means
# production), checks it names a channel this repository has, and sets the
# TC_* bundle values from that channel's file. The cargo features that make
# a staging dylib and CLI arrive through the file's TC_FFI_CARGO_FEATURES and
# TC_CLI_CARGO_FEATURES, so no build script names the feature
# (scripts/ci/check-release-staging-feature.sh scans them).
#
# The file is data, not a script. It is read line by line and never
# evaluated: a value cannot run a command, an unknown key is refused (a
# misspelt key would otherwise leave the real one unset), and a missing key
# is refused. Nothing is printed on success, so info-plist.sh's stdout stays
# the plist.

# The keys every channel file must set, and the only keys it may set.
TC_CHANNEL_KEYS=(
  TC_BUNDLE_ID
  TC_BUNDLE_NAME
  TC_URL_SCHEME
  TC_URL_TYPE_NAME
  TC_SPARKLE_FEED_URL
  TC_PRIVATE_AI_CAMPAIGN_URL
  TC_SPARKLE_KEY_VAR
  TC_ENTITLEMENTS
  TC_PROFILE
  TC_PROFILE_VAR
  TC_ICON_SOURCE
  TC_FFI_CARGO_FEATURES
  TC_CLI_CARGO_FEATURES
)

load_channel() {
  local channels_dir
  channels_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../channels" && pwd)"
  TC_CHANNEL="${TC_CHANNEL:-production}"
  # A fixed list, not a path: the name selects a file, so it must never be
  # able to name one outside the channels directory.
  case "$TC_CHANNEL" in
    production | staging) ;;
    *)
      echo "refusing: TC_CHANNEL must be production or staging" >&2
      return 1
      ;;
  esac
  local file="$channels_dir/$TC_CHANNEL.env"
  if [[ ! -f "$file" ]]; then
    echo "refusing: no channel file for $TC_CHANNEL" >&2
    return 1
  fi

  local key line value seen=" "
  for key in "${TC_CHANNEL_KEYS[@]}"; do
    unset "$key"
  done
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    if [[ ! "$line" =~ ^([A-Z][A-Z0-9_]*)=(.*)$ ]]; then
      echo "refusing: $TC_CHANNEL channel file has a line that is not KEY=VALUE" >&2
      return 1
    fi
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    if [[ " ${TC_CHANNEL_KEYS[*]} " != *" $key "* ]]; then
      echo "refusing: $TC_CHANNEL channel file sets unknown key $key" >&2
      return 1
    fi
    if [[ "$seen" == *" $key "* ]]; then
      echo "refusing: $TC_CHANNEL channel file sets $key twice" >&2
      return 1
    fi
    seen="$seen$key "
    printf -v "$key" '%s' "$value"
  done <"$file"

  for key in "${TC_CHANNEL_KEYS[@]}"; do
    if [[ "$seen" != *" $key "* ]]; then
      echo "refusing: $TC_CHANNEL channel file does not set $key" >&2
      return 1
    fi
    # Only the features may be empty: production adds none.
    if [[ "$key" != TC_*_CARGO_FEATURES && -z "${!key}" ]]; then
      echo "refusing: $TC_CHANNEL channel file leaves $key empty" >&2
      return 1
    fi
  done
  # The bundle name becomes a path the build deletes and recreates: no
  # slash, no leading dot, nothing a shell would read as syntax.
  if [[ ! "$TC_BUNDLE_NAME" =~ ^[A-Za-z0-9][A-Za-z0-9.\ ]*$ ]]; then
    echo "refusing: $TC_CHANNEL channel file's TC_BUNDLE_NAME must be letters, digits, dots and spaces" >&2
    return 1
  fi
  # Variable names, used with ${!name}: refuse anything that is not one.
  for key in TC_SPARKLE_KEY_VAR TC_PROFILE_VAR; do
    if [[ ! "${!key}" =~ ^[A-Z][A-Z0-9_]*$ ]]; then
      echo "refusing: $TC_CHANNEL channel file's $key is not a variable name" >&2
      return 1
    fi
  done
}

# The channel a bundled `near-ai --version` reports: the word after
# "channel " at the end of its line.
near_ai_channel() {
  local line
  line="$("$1" --version)" || return 1
  [[ "$line" =~ \ channel\ ([a-z]+)$ ]] || return 1
  printf '%s\n' "${BASH_REMATCH[1]}"
}

# The channel and URL scheme a bundled FFI library reports through
# tc_build_channel_json, as "<channel> <scheme>". The library is loaded from
# a scratch copy, ad-hoc signed where codesign exists, because the copy in
# the bundle has had its install name rewritten and is not signed yet.
library_channel() {
  local lib="$1" scratch out
  [[ -f "$lib" ]] || return 1
  scratch="$(mktemp -d)" || return 1
  cp "$lib" "$scratch/lib.dylib" || {
    rm -rf "$scratch"
    return 1
  }
  if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - --timestamp=none "$scratch/lib.dylib" >/dev/null 2>&1 || true
  fi
  out="$(python3 -I -c '
import ctypes, json, sys
lib = ctypes.CDLL(sys.argv[1])
lib.tc_build_channel_json.restype = ctypes.c_void_p
lib.tc_string_free.argtypes = [ctypes.c_void_p]
raw = lib.tc_build_channel_json()
if not raw:
    sys.exit(1)
try:
    doc = json.loads(ctypes.string_at(raw).decode("utf-8"))
finally:
    lib.tc_string_free(raw)
print(doc["channel"], doc["url_scheme"])
' "$scratch/lib.dylib" 2>/dev/null)"
  local status=$?
  rm -rf "$scratch"
  ((status == 0)) || return 1
  [[ "$out" =~ ^[a-z]+\ [a-z-]+$ ]] || return 1
  printf '%s\n' "$out"
}

# Refuse a bundle whose Info.plist TCChannel, bundled CLI, bundled FFI
# library and chosen channel disagree, or whose library's URL scheme is not
# the channel's (staging design 4.3). The app makes the same comparison
# against its dylib at launch and starts nothing on a mismatch; checking it
# here stops a bundle that could never start from being signed.
assert_bundle_channel() {
  local app="$1" plist_channel cli_channel lib_report lib_channel lib_scheme
  plist_channel="$(python3 -I -c '
import plistlib, sys
with open(sys.argv[1], "rb") as f:
    print(plistlib.load(f)["TCChannel"])
' "$app/Contents/Info.plist" 2>/dev/null)" || {
    echo "FATAL: the bundle's Info.plist has no TCChannel" >&2
    return 1
  }
  cli_channel="$(near_ai_channel "$app/Contents/MacOS/near-ai")" || {
    echo "FATAL: the bundled near-ai does not report its channel in --version" >&2
    return 1
  }
  if [[ "$plist_channel" != "$TC_CHANNEL" || "$cli_channel" != "$TC_CHANNEL" ]]; then
    echo "FATAL: channel mismatch: chosen $TC_CHANNEL, Info.plist $plist_channel, near-ai $cli_channel" >&2
    return 1
  fi
  lib_report="$(library_channel "$app/Contents/Frameworks/libtrace_commons_contributor_ffi.dylib")" || {
    echo "FATAL: the bundled FFI library does not report its channel" >&2
    return 1
  }
  lib_channel="${lib_report%% *}"
  lib_scheme="${lib_report#* }"
  if [[ "$lib_channel" != "$TC_CHANNEL" ]]; then
    echo "FATAL: channel mismatch: chosen $TC_CHANNEL, FFI library $lib_channel" >&2
    return 1
  fi
  if [[ "$lib_scheme" != "$TC_URL_SCHEME" ]]; then
    echo "FATAL: URL scheme mismatch: $TC_CHANNEL channel file $TC_URL_SCHEME, FFI library $lib_scheme" >&2
    return 1
  fi
  echo "verified channel $TC_CHANNEL: Info.plist, near-ai and the FFI library agree"
}
