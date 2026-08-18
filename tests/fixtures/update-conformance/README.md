# Update conformance fixtures

The verify-before-swap logic for automatic updates exists twice: once in Rust
(`crates/trace-commons-contributor/src/update/`) and once in Swift for the
macOS app. These fixtures are the mitigation for that duplication. Both suites
read these exact bytes, so a check dropped in either implementation fails a
test rather than shipping.

| Path | What it is | What must happen |
|---|---|---|
| `good/latest.json` + `.sig` | correctly signed, version `9.9.9` | verifies; is newer than any real build |
| `good/artifact.bin` | the artifact the good manifest publishes | sha256 and size match the manifest |
| `tampered/artifact.bin` | same length, different bytes | digest check refuses it |
| `bad-signature/latest.json` + `.sig` | signed by `wrong-signing-key.pem` | signature check refuses it |
| `downgrade/latest.json` + `.sig` | correctly signed, version `0.0.1` | verifies, then the version gate refuses it |
| `unsigned/artifact.exe` | a blob with no Authenticode signature | the Windows signature check refuses it |
| `manifest-public-key.hex` | raw 32-byte Ed25519 public key | what a client pins in these tests |

`signing-key.pem` and `wrong-signing-key.pem` are committed private keys. They
are test fixtures built from fixed seeds, they sign nothing that is ever
published, and they are committed so both suites can re-derive the same
signatures.

Regenerate with `./regenerate.sh`. It is deterministic: keys come from fixed
seeds and manifests carry a fixed `published_at`, so re-running changes
nothing unless a fixture genuinely changed.
