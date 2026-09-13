# Comparison configuration encoding v1

The stored configuration fingerprint is SHA-256 over these canonical bytes:

1. ASCII domain separator `trace-commons-comparison-configuration-v1` followed by one NUL byte.
2. Harness ID, harness version, reasoning effort, tool-policy ID, tool-policy version, and prompt-template digest, in that order.
3. Each optional string is one tag byte (`0` unknown, `1` known). A known value continues with its UTF-8 byte length as unsigned 32-bit big-endian, then its exact bytes.
4. Reasoning effort is one byte: unknown `0`, none `1`, minimal `2`, low `3`, medium `4`, high `5`, xhigh `6`.

Known labels are case-sensitive ASCII, 1–128 bytes, using letters, digits, `.`, `_`, `:`, `/`, `+`, or `-`. Digests are exactly 64 lowercase hexadecimal characters. Models and providers are absent.

The golden configuration `codex`, `0.12.2`, medium, `direct-v1`, `1`, and prompt digest `11` repeated 32 times hashes to `8c24d6be0b933a0aab1876834b73ec46229aa7e4290895c06d8c4de5dda3b9aa`. The unit test pins the complete encoded byte string.

Checkout provenance is `unavailable` in v1. No tree-digest construction has yet been qualified.
