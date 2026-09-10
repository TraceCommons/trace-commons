# Token distribution implementation progress

Working branch: `implement-token-logprobs`, based on Trace Commons main `cbb27dff`.
The [implementation plan](2026-09-10-token-logprobs-lifecycle.md) remains the full scope.

## Implemented locally

- Permissive protocol types for raw and sanitized token records, byte spans, model/conditioning metadata, omission counts, restricted-use manifests, and durable receipt identities.
- Bounded decoding, exact response reconstruction, canonical manifest encoding and digest binding, finite FP64 preservation, explicit unavailable/negative-infinity values, and strict version handling.
- A private edit-map filtering primitive: remove entire overlapping token records, require contextual alternative decisions, omit uncertain positions, shift retained offsets, and preserve provider probabilities. This is a primitive, not a PII detector or a provider verifier.
- Sanitized attachment validation against event bytes and policy. Changed text after ingest rescrubbing invalidates the attachment association.
- Read-only extraction from complete buffered and SSE Chat Completions bodies, including final-frame probabilities and byte-split UTF-8. Tools, reasoning, partial streams, and unsupported framing remain unavailable. This parser does not authenticate provider responses.
- Receipt matching binds server, tenant, account, revision, manifest digest, and retention interval. It does not authorize deletion by itself; authenticated response handling and durable local journaling must precede cleanup.

V1 attachments are uncompressed JSON, one bounded attachment per segment. Finite probabilities use decimal strings with scientific notation and correctly rounded FP64 parsing. This avoids serde_json feature unification changing a round-tripped value. Full chunk/codec negotiation and per-exchange attestation remain integration work.

## Verified

- Protocol tests with `RUSTFLAGS=-D warnings`, standalone/no default features.
- Protocol tests with `serde_json/preserve_order` enabled during contract validation.
- Protocol clippy with the repository allow-list; formatting.
- Server license-boundary test (all four cases pass).
- No dependencies changed. No capture/upload behavior enabled, and no deployment performed.

## Remaining work (not implemented)

1. P0 live provider/model/receipt qualification. Synthetic parser tests are not live capability evidence.
2. P1 final certificate, consent, chunk/codec, and lifecycle contracts, including cross-language fixtures.
3. P2/P3 upstream Ironwire opt-in request behavior, bounded spool, durable leases, GC coordination, and control/embedded APIs. An isolated `capture-token-distributions` worktree exists at `/tmp/ironwire-token-logprobs`; it currently has no source changes.
4. P4 redactors emitting composed edit maps, contextual alternatives policy, exact source verification, bounded witness processing, and signed bundle transport.
5. P5/P6 encrypted binary artifact storage, PostgreSQL/RLS migrations, staged uploads, durable finalization/receipt lookup, rescrub derivatives, and orphan recovery.
6. P7 client snapshot/approval/transfer integration and receipt-plus-cleanup journal.
7. P8 restricted indexes/exports and immutable lineage.
8. P9 lease release worker, withdrawal/retention traversal, concurrency and restore qualification.
9. P10/P11 platform controls, merged upstream dependency pin, release checks and restricted pilot.

The feature is not complete or available to users. Continue with provider qualification and the remaining shared contracts; do not interpret passing protocol tests as end-to-end readiness.
