#!/bin/sh
# Deliberately does not start anything.
#
# IronWire holds the user's subscription credentials, and a package install is
# not consent to begin using them (docs/TRUST.md §2). It also cannot be: a
# user unit has no meaning during a root-run postinst. So this prints what to
# run and stops.
set -e

cat <<'MSG'

IronWire installed.

  ironwire connect claude     point Claude Code at it
  ironwire connect codex      point Codex at it
  ironwire serve              run it in the foreground

To run it in the background:

  systemctl --user enable --now ironwire

IronWire binds 127.0.0.1 only and keeps everything under ~/.ironwire.
No subscription is used until you enable it explicitly.

MSG
