#!/usr/bin/env bash
# Validate the GTK shell's desktop entry and AppStream metainfo, and pin the
# metainfo's newest release to the crate version.
#
# One copy, run by two CI jobs: `linux-shell desktop entry and metainfo`
# (fast, answers in about a minute) and `linux-shell desktop integration
# (weston + portal)` (the required check, where it used to be inline). The
# weston copy stays until the fast job is itself a required check; a
# validation that only ran in a non-required job would still run and no
# longer block. Needs `desktop-file-validate` (desktop-file-utils) and
# `appstreamcli` (appstream) on PATH.
set -euo pipefail

cd "$(dirname "$0")/../../.."
flatpak=crates/trace-commons-contributor-gtk/flatpak

desktop-file-validate "$flatpak/ai.tracecommons.Contributor.desktop"
appstreamcli validate --no-net --pedantic "$flatpak/ai.tracecommons.Contributor.metainfo.xml"

# The entry names an icon; the manifest must install a file under that
# name. Checked as a pair because each half looks fine alone.
icon_name="$(sed -n 's/^Icon=//p' "$flatpak/ai.tracecommons.Contributor.desktop")"
grep -q "apps/${icon_name}.svg" "$flatpak/ai.tracecommons.Contributor.yml" \
  || { echo "the flatpak manifest installs no icon named ${icon_name}" >&2; exit 1; }

# The newest <release> must name the version the crate is at. Nothing else
# checks this: appstreamcli validates the document's shape and cannot know
# what the crate calls itself, so 0.10.0 was tagged with a metainfo still
# naming the previous release and was caught only by a person reading it
# before the tag. The version is not covered by the workspace either --
# this crate carries its own.
crate_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' \
  crates/trace-commons-contributor-gtk/Cargo.toml | head -1)"
meta_version="$(sed -n 's/.*<release version="\([^"]*\)".*/\1/p' \
  "$flatpak/ai.tracecommons.Contributor.metainfo.xml" | head -1)"
[ -n "$crate_version" ] && [ -n "$meta_version" ] \
  || { echo "could not read one of the two versions" >&2; exit 1; }
[ "$crate_version" = "$meta_version" ] \
  || { echo "metainfo newest release is ${meta_version}, crate is ${crate_version}" >&2; exit 1; }
echo "desktop entry, metainfo, icon pair and version pin (${crate_version}) all agree"
