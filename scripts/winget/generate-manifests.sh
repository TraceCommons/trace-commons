#!/usr/bin/env bash
# Generate the winget manifests for a published CLI release.
#
#   scripts/winget/generate-manifests.sh 0.1.1 [asset-url]
#
# Writes the three-file manifest set that the Windows Package Manager community
# repository expects, under the partitioned path it expects:
#
#   manifests/t/TraceCommons/Contributor/<version>/
#     TraceCommons.Contributor.yaml               (version)
#     TraceCommons.Contributor.locale.en-US.yaml  (defaultLocale)
#     TraceCommons.Contributor.installer.yaml     (installer)
#
# The InstallerSha256 is computed from the artifact this script downloads, never
# typed in. A wrong hash in a winget manifest is rejected by their CI at best and
# ships a package that refuses to install at worst, and it is exactly the kind of
# value that gets copied from the wrong release.
#
# Why zip and not the bare .exe: winget can only name the installed command for
# a nested installer inside an archive -- PortableCommandAlias is defined only
# within NestedInstallerFiles in the 1.12.0 installer schema. Pointed at the
# bare .exe, `winget install` would create a command named
# trace-commons-contributor-x86_64-pc-windows-msvc. The release publishes a zip
# holding the signed binary under its plain name for exactly this reason.
set -euo pipefail

VERSION="${1:?usage: generate-manifests.sh <version> [asset-url]   e.g. 0.1.1}"
TAG="contributor-v$VERSION"
REPO="TraceCommons/trace-commons"
ASSET="trace-commons-contributor-x86_64-pc-windows-msvc.zip"
URL="${2:-https://github.com/$REPO/releases/download/$TAG/$ASSET}"

OUT="manifests/t/TraceCommons/Contributor/$VERSION"
MANIFEST_VERSION="1.12.0"

command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "release : $TAG"
echo "asset   : $URL"

if ! curl -fsSL --proto '=https' --tlsv1.2 "$URL" -o "$tmp/$ASSET"; then
  cat >&2 <<EOF
could not download the Windows zip for $TAG.

If the release exists but has no .zip, it was cut before the release workflow
started publishing one. winget needs the archive, not the bare .exe -- see the
note at the top of this script. Cut a release that includes it, or pass an
explicit asset URL as the second argument.
EOF
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  SHA="$(sha256sum "$tmp/$ASSET" | awk '{print $1}')"
else
  SHA="$(shasum -a 256 "$tmp/$ASSET" | awk '{print $1}')"
fi
# winget-pkgs writes these uppercase.
SHA="$(printf '%s' "$SHA" | tr '[:lower:]' '[:upper:]')"

# The archive must contain the plain binary name the alias maps to. Checking
# here means a rename in the release workflow surfaces now rather than as a
# winget install that resolves to nothing.
if command -v unzip >/dev/null 2>&1; then
  if ! unzip -l "$tmp/$ASSET" | grep -q 'trace-commons-contributor\.exe'; then
    echo "the zip does not contain trace-commons-contributor.exe:" >&2
    unzip -l "$tmp/$ASSET" >&2
    exit 1
  fi
fi

# The release's own publication date, not today's. Generating or regenerating a
# manifest for an older release would otherwise stamp it with the day the script
# happened to run. If the API cannot tell us, the field is omitted entirely --
# an absent optional field beats a confidently wrong one.
RELEASE_DATE="$(curl -fsSL --proto '=https' --tlsv1.2 \
    "https://api.github.com/repos/$REPO/releases/tags/$TAG" 2>/dev/null \
  | grep -o '"published_at":[[:space:]]*"[^"]*"' \
  | head -1 \
  | sed -e 's/.*"\([0-9-]\{10\}\)T.*/\1/')" || RELEASE_DATE=""

if [ -n "$RELEASE_DATE" ]; then
  # Quoted, because an unquoted YYYY-MM-DD is a YAML date, and the schema wants a
  # string. Found by validating the output against winget's own JSON schema.
  RELEASE_DATE_LINE="ReleaseDate: \"$RELEASE_DATE\""
else
  RELEASE_DATE_LINE=""
fi

mkdir -p "$OUT"

cat > "$OUT/TraceCommons.Contributor.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.$MANIFEST_VERSION.schema.json
PackageIdentifier: TraceCommons.Contributor
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $MANIFEST_VERSION
EOF

cat > "$OUT/TraceCommons.Contributor.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$MANIFEST_VERSION.schema.json
PackageIdentifier: TraceCommons.Contributor
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: Iqlusion Inc
PublisherUrl: https://tracecommons.ai
PublisherSupportUrl: https://github.com/$REPO/issues
PackageName: Trace Commons Contributor
PackageUrl: https://tracecommons.ai/install/
License: MIT OR Apache-2.0
LicenseUrl: https://github.com/$REPO/blob/main/LICENSE
ShortDescription: Share the traces your coding agents produce, on your terms, and earn credits.
Description: |-
  The Trace Commons contributor CLI finds the sessions your local coding agents
  have already written, shows you exactly what would be sent before anything is,
  and submits only what you approve. Discovery, parsing and redaction all happen
  on your own machine; nothing leaves it until you run submit.
Moniker: trace-commons
Tags:
- ai
- agent
- cli
- developer-tools
- llm
- traces
ReleaseNotesUrl: https://github.com/$REPO/releases/tag/$TAG
Documentations:
- DocumentLabel: Quickstart
  DocumentUrl: https://docs.tracecommons.ai/cli/quickstart/
ManifestType: defaultLocale
ManifestVersion: $MANIFEST_VERSION
EOF

cat > "$OUT/TraceCommons.Contributor.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.$MANIFEST_VERSION.schema.json
PackageIdentifier: TraceCommons.Contributor
PackageVersion: $VERSION
Platform:
- Windows.Desktop
MinimumOSVersion: 10.0.0.0
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: trace-commons-contributor.exe
  PortableCommandAlias: trace-commons-contributor
Commands:
- trace-commons-contributor
$RELEASE_DATE_LINE
Installers:
- Architecture: x64
  InstallerUrl: $URL
  InstallerSha256: $SHA
ManifestType: installer
ManifestVersion: $MANIFEST_VERSION
EOF

echo ""
echo "wrote:"
for f in "$OUT"/*.yaml; do echo "  $f"; done
echo ""
echo "sha256: $SHA"
echo ""
cat <<EOF
Next, to submit:
  1. Fork and clone https://github.com/microsoft/winget-pkgs
  2. Copy $OUT into the fork at the same path
  3. Validate:  winget validate --manifest <path>   (on Windows)
     and ideally test: winget install --manifest <path>
  4. Open a pull request against microsoft/winget-pkgs

Only x64 is declared, because that is the only Windows architecture we publish.
Windows on Arm runs it under x64 emulation.
EOF
