#!/bin/sh
# One-time Trace Commons contribution: fetch the verified CLI, submit the
# sessions under the directory you are standing in, and exit.
#
#   # from a project directory: that project
#   # from a parent of several repos: all of them
#   TRACE_COMMONS_INVITE='<your invite link>' \
#     curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/contribute.sh | sh
#
# Reading it before running it is encouraged, and the two-step form is
# documented first in the README for exactly that reason. The security
# boundary is not this file: verification lives in scripts/install.sh, which
# refuses any binary whose published SHA-256 does not match and, on macOS, any
# whose signature does not name our Developer ID. A tampered copy of this
# script cannot talk that one into accepting an unsigned binary.
#
# WHAT THIS LEAVES BEHIND
#
# The binary is ephemeral: it goes in a cache directory, not on your PATH, and
# no daemon, autostart, or login item is created. But one thing does persist,
# deliberately -- the keep: a device key and the coordinates needed to use it.
#
# The device key IS your identity here, not merely a credential for it. Your
# account is minted from it. Delete it and there is no way to sign in, and
# therefore no way to withdraw the traces this run uploads. A one-time script
# that left a contributor unable to retract what they sent would be a consent
# failure, so there is no flag that suppresses the keep. Deleting it afterwards
# is the same act with better timing: by then you have seen what it is for.
#
# The invite is never passed in argv, where it would land in your shell
# history and in `ps` for every user on the machine. Set TRACE_COMMONS_INVITE,
# or let the script prompt for it on a terminal.
#
# The keep is deliberately its OWN directory, not the state directory an
# installed CLI uses. Two reasons. A script fetched over the network should not
# silently adopt an existing enrollment and submit under an identity you did
# not pick for it; and the delete instruction below has to be safe to follow,
# which `rm -rf` on an installed CLI's state would not be.
#
# The cost is worth stating plainly: if you later install the CLI properly, it
# enrolls separately. That is a second device key, so a second identity, a
# second invite use, and per-contributor credit that does not add up across the
# two. Point TRACE_COMMONS_KEEP_DIR at your installed state directory if you
# would rather they were one -- and then do not run the delete command.

set -eu

# Everything is inside this function, invoked on the very last line. A
# truncated download therefore executes nothing at all -- table stakes once a
# script is advertised in its piped form.
tc_contribute_main() {
  REPO_RAW="https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main"

  # XDG-ish, with plain fallbacks. Both are ordinary user-owned directories;
  # nothing here needs sudo and nothing is written outside them.
  cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/tracecommons"
  bin_dir="$cache_root/bin"
  keep_dir="${TRACE_COMMONS_KEEP_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/tracecommons/keep}"

  say() { printf '%s\n' "$*"; }
  die() { printf 'contribute failed: %s\n' "$*" >&2; exit 1; }
  need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

  need curl
  need mkdir

  # ---- refuse the unbounded run early ---------------------------------
  # The CLI refuses this too (see `resolve_submit_scope`), but refusing here
  # means we do not download anything on the way to an error.
  cwd="$(pwd -P)"
  home="$(cd "$HOME" 2>/dev/null && pwd -P)" || home=""
  case "$cwd" in
    /) die "run this from a project directory. From a filesystem root the
scope would be every session on this machine." ;;
  esac
  if [ -n "$home" ] && [ "$cwd" = "$home" ]; then
    die "run this from a project directory, or a parent of the repos you want
to contribute. From \$HOME the scope would be every session on this machine."
  fi

  # ---- the invite, never from argv ------------------------------------
  invite="${TRACE_COMMONS_INVITE:-}"
  if [ -z "$invite" ]; then
    # `< /dev/tty` matters: stdin is the piped script itself.
    if [ -r /dev/tty ]; then
      printf 'invite link: ' > /dev/tty
      IFS= read -r invite < /dev/tty || invite=""
    fi
  fi
  if [ -z "$invite" ] && [ ! -d "$keep_dir" ]; then
    die "no invite. Set TRACE_COMMONS_INVITE to the link you were handed, or
run this from a terminal so it can ask."
  fi

  # ---- fetch the verified binary --------------------------------------
  mkdir -p "$bin_dir" || die "could not create $bin_dir"
  cli="$bin_dir/trace-commons-contributor"

  installer="$(mktemp)" || die "could not create a temporary file"
  # shellcheck disable=SC2064
  trap "rm -f '$installer'" EXIT INT TERM
  curl -fsSL --proto '=https' --tlsv1.2 "$REPO_RAW/scripts/install.sh" -o "$installer" \
    || die "could not download the installer"
  # install.sh owns checksum and signature verification, and has no --force
  # and no --skip-verify. Reimplementing either here would be a second chance
  # to get verification wrong.
  sh "$installer" --dir "$bin_dir" || die "the CLI could not be verified and installed"
  [ -x "$cli" ] || die "the installer did not produce $cli"

  # ---- the keep --------------------------------------------------------
  mkdir -p "$keep_dir" || die "could not create $keep_dir"
  chmod 700 "$keep_dir" 2>/dev/null || true
  TRACE_COMMONS_CONTRIBUTOR_DIR="$keep_dir"
  export TRACE_COMMONS_CONTRIBUTOR_DIR

  say ""
  say "keep: $keep_dir"
  say "  This holds the device key your account is minted from. It is the only"
  say "  way to sign in and withdraw the traces this run uploads."
  say "  To delete it later, and give up the ability to withdraw them:"
  say "    rm -rf \"$keep_dir\""
  say ""

  # ---- one submission --------------------------------------------------
  # `submit` scopes itself to this directory's subtree and enrolls with the
  # invite only if the keep holds no config yet -- so a second run finds the
  # keep and spends no further invite use.
  #
  # The invite goes through the environment, never on the command line: argv
  # is visible to every user on the machine via `ps`.
  #
  # Stdin is the piped script, so the confirmation and the consent questions
  # must read the terminal instead -- otherwise they would consume script text
  # as their answers. With no terminal there is nobody to answer them, and a
  # run that uploaded traces unconfirmed would be the failure this prompt
  # exists to prevent, so it stops.
  if [ ! -r /dev/tty ]; then
    die "no terminal to confirm the upload on. Install the CLI with
scripts/install.sh and run \`trace-commons-contributor submit\` yourself."
  fi
  status=0
  TRACE_COMMONS_INVITE="$invite" "$cli" submit < /dev/tty || status=$?

  say ""
  say "keep: $keep_dir  (the only way to withdraw these traces)"
  say "  withdraw : TRACE_COMMONS_CONTRIBUTOR_DIR=\"$keep_dir\" \"$cli\" account login"
  say "  delete   : rm -rf \"$keep_dir\""
  return $status
}

tc_contribute_main "$@"
