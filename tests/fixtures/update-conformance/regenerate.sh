#!/usr/bin/env bash
# Regenerate the shared update-conformance fixtures.
#
# Deterministic on purpose: the Ed25519 keys are built from fixed seeds and
# the manifests carry a fixed timestamp, so re-running this produces byte
# identical output and a regeneration shows up in a diff only when a fixture
# actually changed.
#
# These keys are test fixtures. They sign nothing that is ever published, and
# the private keys are committed deliberately so that both the Rust and the
# Swift test suites can re-derive the same signatures.
set -euo pipefail

cd "$(dirname "$0")"

die() { echo "regenerate: $*" >&2; exit 1; }
command -v openssl >/dev/null || die "openssl is required"
command -v xxd >/dev/null || die "xxd is required"
command -v jq >/dev/null || die "jq is required"

# An Ed25519 private key in PKCS#8 v1 is a fixed 16-byte DER prefix followed
# by the 32-byte seed, so a chosen seed yields a reproducible PEM.
write_key() {
  seed_hex="$1"
  out="$2"
  printf '302e020100300506032b657004220420%s' "$seed_hex" | xxd -r -p > "$out.der"
  {
    printf -- '-----BEGIN PRIVATE KEY-----\n'
    openssl base64 -in "$out.der"
    printf -- '-----END PRIVATE KEY-----\n'
  } > "$out"
  rm -f "$out.der"
}

write_key "$(printf '2a%.0s' $(seq 1 32))" signing-key.pem
write_key "$(printf '5b%.0s' $(seq 1 32))" wrong-signing-key.pem

# The raw 32-byte public key clients pin is the tail of the DER
# SubjectPublicKeyInfo.
openssl pkey -in signing-key.pem -pubout -outform DER \
  | tail -c 32 | xxd -p -c 32 > manifest-public-key.hex

mkdir -p good bad-signature downgrade tampered unsigned

printf 'trace-commons update conformance good artifact\n' > good/artifact.bin
# Same length, different bytes: the tampered case must fail on the digest and
# not incidentally on the size.
printf 'trace-commons update conformance EVIL artifact\n' > tampered/artifact.bin
printf 'not a signed windows binary\n' > unsigned/artifact.exe

SHA="$(openssl dgst -sha256 -hex good/artifact.bin | awk '{print $NF}')"
SIZE="$(wc -c < good/artifact.bin | tr -d ' ')"

write_manifest() {
  version="$1"
  out="$2"
  # Every CLI slug points at the same artifact so the fixtures exercise the
  # client on whatever host the test runs on.
  platforms=""
  for slug in windows-x86_64-cli linux-x86_64-cli macos-aarch64-cli macos-x86_64-cli; do
    entry="$(printf '"%s":{"url":"https://example.invalid/%s","sha256":"%s","size":%s}' \
               "$slug" "$slug" "$SHA" "$SIZE")"
    if [ -n "$platforms" ]; then platforms="$platforms,$entry"; else platforms="$entry"; fi
  done
  printf '{"schema_version":"trace_commons.update_manifest.v1","version":"%s","published_at":"2026-08-17T00:00:00Z","platforms":{%s}}' \
    "$version" "$platforms" | jq -S . > "$out"
}

sign() {
  key="$1"; in="$2"
  openssl pkeyutl -sign -rawin -inkey "$key" -in "$in" | openssl base64 -A > "$in.sig"
}

# The good case: a version no released build will ever reach, so it is always
# strictly newer than whatever is running.
write_manifest 9.9.9 good/latest.json
sign signing-key.pem good/latest.json

# A well-formed manifest signed by a key clients do not pin.
write_manifest 9.9.9 bad-signature/latest.json
sign wrong-signing-key.pem bad-signature/latest.json

# A correctly signed manifest for an old version. Replaying this at a client
# is a working attack unless the version comparison stops it, which is why
# this fixture exists separately from the bad-signature one.
write_manifest 0.0.1 downgrade/latest.json
sign signing-key.pem downgrade/latest.json

echo "regenerated fixtures in $(pwd)"
