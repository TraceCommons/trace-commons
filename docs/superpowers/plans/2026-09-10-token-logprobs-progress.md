# Token distribution implementation status

Implementation work is isolated on `implement-token-logprobs`. The upstream
capture implementation is [nearai/ironwire#57](https://github.com/nearai/ironwire/pull/57),
pinned at `4652f0481767c785e9d3a21ba180a2f4fa11295d` in both client lockfiles and
the regenerated Flatpak sources. The Ironclaw revision and registry package
versions are unchanged.

The [full plan](2026-09-10-token-logprobs-lifecycle.md) remains the acceptance
specification. This implementation covers the initial, explicitly restricted
profile: one verified final Chat Completions exchange, JSON or complete SSE.
Responses API, tools, reasoning, unverified companion history, incomplete
streams and unqualified backend/model pairs remain unavailable.

## Integrated behavior

- Ironwire defaults detailed capture off, adds only absent probability request
  fields for configured pairs, and preserves exact upstream response bytes.
  Private bounded spools expose authenticated immutable snapshots, owner-bound
  leases, renewal, release and garbage collection.
- The witness verifies the provider receipt and admission challenge before
  deriving an assistant event from the verified exchange. Actual deterministic
  and classifier edits compose into private byte maps; unchanged token spans
  survive. Missing provenance drops the segment. Alternatives are checked in
  sanitized context, with secret fragments, classifier failures and resource
  limits handled conservatively. Exact envelope and manifest bytes are signed.
- Explicit desktop review negotiates ingest support, selects one exact capture
  digest match, obtains a lease, verifies the witness, and pins the signed
  payload. The review shows included/omitted positions and alternative counts.
  Approval and retry use the stored bytes without re-running the witness.
- Capture configuration, inference-body disclosure and contribution permission
  are separate. macOS, Windows and GTK expose contribution controls with shared
  Rust wording; CLI uses the same saved setting. Incompatible consent changes
  invalidate queued approvals instead of silently changing the upload.
- Begin/upload/finalize persist encrypted staging packets, read back every
  object, and return a destination-bound durable receipt only after completion.
  The exact certified envelope is stored too. PostgreSQL enforces immutable
  revisions, owner/tenant isolation, forced RLS and staging/active quotas.
- Owner-only restricted downloads validate current event correspondence and
  register export lineage before returning bytes. They identify the export and
  manifest digest in response headers. Ordinary text/vector/corpus exports do
  not receive token attachments; broader researcher access stays disabled.
- The client journals acknowledgement before releasing its own lease. Retries
  recover after acknowledgement, release or payload cleanup. Discarded reviews
  abandon their payloads; pending reviews expire after three days, approved
  ones after seven. Approved leases renew in bounded batches with backoff.
  Local envelope and bundle writers share a locked 256 MiB budget. Logout
  clears review data; agent session files are never removed.
- Parent revocation blocks reads and new revisions. Publication/finalization
  and deletion use database locks, and garbage collection observes both parent
  tombstones and withdrawal records. Cleanup retries are idempotent.

## Qualification evidence

- Upstream PR #57 CI passes on macOS and Ubuntu, including packaging, journey
  and size checks.
- Protocol tests cover exact bytes, split Unicode, decimal probabilities,
  composed deterministic/classifier edits, repeated text and absent provenance.
- Signed witness testing verifies receipt-bound event projection, manifest and
  envelope certificates, byte correspondence, missing consent and tampering.
  Client wire testing confirms isolated-call projection and no ordinary fallback.
- Alternative screening tests cover removed secret fragments, shifted spans,
  classifier outage and bounded context; values are not renormalized.
- PostgreSQL qualification uses an actual NOBYPASSRLS role and tests isolation,
  concurrent retries, immutable staging, quotas, incomplete commits, ciphertext
  publication before ready-state recovery, concurrent commits, parent revocation,
  deletion retries and a new revision racing the parent lifecycle lock.
- macOS: 847 XCTest cases and 8 Swift Testing cases passed locally. GTK: 517
  unit tests plus integration tests passed locally. Windows interop: 994 passed,
  one native-Windows case skipped on macOS; the available .NET runtime was used
  with major-version roll-forward. These are not Windows/Wayland native UI runs.
- All four required cargo-deny license configurations pass. No new dependency
  packages were added. Full-workspace and final clippy checks are recorded in
  the PR validation; targeted runtime tests supplement them.
- `scripts/token-distributions/qualify.py` supplies a bounded synthetic provider
  probe and records metadata/timing only. Its parser tests pass. No live provider
  result is inferred from those fixtures.

## Gates before enabling production

1. Review and merge upstream #57, then confirm the downstream pin and platform
   CI on the final commits. Native Windows packaging/runtime qualification is
   still required.
2. Run the synthetic probe against each intended provider/model and record its
   attested serving identity, probability semantics and overhead. Populate no
   default qualified target from a model alias or fixture alone.
3. Qualify real object-store timeout/versioning behavior, backup restoration
   with withdrawal tombstones, and production cleanup/retention operations.
   Local encrypted-file tests do not establish cloud-version or backup erasure.
4. Roll out the changed witness through the existing measurement/pin procedure,
   then conduct the bounded pilot. The implementation has not changed deployed
   witness pins, enabled capture, or enabled production bundle receipts.

Broader researcher exports, additional protocol profiles and inference-wide
multi-call coverage require their own qualified expansion. None is silently
represented as covered by this initial profile.
