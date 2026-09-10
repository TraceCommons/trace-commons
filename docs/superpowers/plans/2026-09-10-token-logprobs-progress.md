# Token distribution implementation progress

Trace Commons worktree: `/tmp/tc-token-logprobs`, branch `implement-token-logprobs`.
Ironwire worktree: `/tmp/ironwire-token-logprobs`, branch `capture-token-distributions`.
The [implementation plan](2026-09-10-token-logprobs-lifecycle.md) remains the full scope.

## Implemented locally

- Protocol: bounded raw/sanitized token records, exact byte reconstruction, FP64-preserving probabilities, edit-map filtering, immutable manifests and destination-bound durable receipts. Complete Chat Completions JSON/SSE extraction; unsupported tools/reasoning/incomplete streams fail closed.
- Ironwire: separate opt-in backend/model targets, caller-preserving Chat request augmentation, exact request/response capture, capped private SQLite/file spool, authenticated list/acquire/read/renew/release control API and embedded maintenance. Multi-owner leases protect exact snapshots; capture pressure refuses new captures; renewals cannot exceed seven days from capture.
- Witness: explicit token-bundle route with source receipt verification, admission restriction/evidence, classifier-required transcript filtering, bounded contextual alternative checks, and signatures over exact envelope and manifest bytes. Changed segments conservatively lose all token records until composed pipeline edit maps are available. Only one verified Chat completion is supported.
- Storage: scoped encrypted binary artifacts and idempotent prepared ciphertext publication. V64 adds immutable tenant/principal/revision records, forced RLS, attachment identities, staging expiry, and withdrawal-triggered revocation. Pending ciphertext is retained in PostgreSQL until object publication/readback succeeds, then removed from the row.
- Ingest: gated begin/put/finalize/status and owner-only attachment reads. The exact certified envelope is stored alongside token attachments; a rescrubbed processing envelope cannot stand in for it. Finalize checks event correspondence and requires all artifacts durable before committing a receipt. Withdrawals and retention invoke bundle deletion; object publication/deletion share database locks.
- Client library: loopback-only authenticated capture access, certificate-verifying bundle upload with missing-artifact retries, destination-bound receipt journal, persisted approval payloads, and restart-safe lease-release intents. The daemon retries one acknowledged intent at startup and every five minutes, bound to the original spool identity. Raw bodies are sent only through a verified witness handle. Neither raw captures nor token payload types implement Debug.

These are local implementation components, not a shipped or enabled feature. The contributor still pins the previously reviewed Ironwire revision. No live deployment, trust-pin change, or provider capability claim has been made.

## Verification so far

- Protocol contract/parser tests and license-boundary tests passed in the foundation pass.
- Ironwire core, ledger and proxy suites passed; all-target clippy passes. Five spool lifecycle tests and 14 passthrough tests pass, including bounded renewal, expired-release retry and exact-wire capture.
- Client receipt journal restart, refusal, retry and path tests pass.
- Encrypted binary artifact replay, tenant-scope refusal and idempotent deletion test passes.
- V64 ran in a fresh disposable PostgreSQL database. Staging immutability, owner/tenant lookup isolation and incomplete-commit refusal test passes. This is not yet a full concurrent RLS/fault-injection test.
- Contributor library suite passed on the workspace rerun (the initial sandboxed run hit a home-directory fixture restriction and a timing-sensitive existing test).
- Workspace testing found the new migration missing from the RLS coverage tests' migration readers; V64 and those readers were corrected. The workspace rerun passed with four test threads. Final focused client tests pass (31 matched tests), fresh PostgreSQL staging/quota tests pass, and server/contributor all-target clippy passes with the existing allowlist. Some later additive helpers received compile/clippy checks rather than another entire workspace run.

## Remaining acceptance work

1. Connect capture selection, bundle preview/approval persistence and upload to the desktop queue; implement renewal scheduling and user controls across macOS/Windows/Linux.
2. Add composed redaction maps so unchanged token spans survive an edited segment; qualify alternative policy with PII attack fixtures and signed end-to-end witness tests.
3. Complete concurrent PostgreSQL RLS, finalize/withdraw/crash/orphan, transport retry, rescrub and backup-restore tests. Validate deployed object-version/deletion semantics before advertising cleanup-safe production receipts.
4. Complete restricted export/index metadata policy and lifecycle propagation; normal text/vector paths currently receive no token attachments.
5. Qualify actual provider/model behavior and overhead with synthetic probes; finish upstream integration/pinning and release checks. Empty target lists and independent server/policy gates keep the feature unavailable by default.

Do not claim end-to-end completion until these acceptance gates pass together.
