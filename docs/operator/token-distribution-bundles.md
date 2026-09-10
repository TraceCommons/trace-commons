# Token distribution bundles (implementation preview)

This feature is disabled by default and is not qualified for production. The client desktop queue and live provider qualification remain incomplete. Do not enable it merely because its unit tests pass.

The current implementation accepts one attested Chat Completions response per bundle. It preserves provider token bytes, chosen-token probabilities and screened alternatives without renormalization. Full-vocabulary logits, Responses API, tools, reasoning and incomplete streams are unavailable. Any transcript edit currently removes all token records for that completion; unchanged completions require the configured classifier to screen alternatives in context. Retained distributions are restricted research data, not anonymous data.

## Independent controls

Ironwire's `capture.token_capture` configuration has an empty target list by default. Enabling it requires ordinary capture to be enabled and explicit backend/model targets. The initial limits are top-20 alternatives, 512 MiB capture storage and three-day unleased retention. Leases may be renewed but never beyond seven days from capture. A full spool refuses new captures and keeps active leases.

The server additionally requires `TRACE_COMMONS_BUNDLE_SERVER_ID`, service-owned encrypted object storage, PostgreSQL bundle support, and an explicit allowlist entry for witness policy `token-distribution-restricted-v1`. The witness requires configured provider admission trust and a classifier-backed redaction pipeline. Deploying a changed witness requires the existing measurement/pin rollout; this implementation does not update deployment pins.

## API and durability

- `POST /v1/token-bundles`: witness-signed canonical manifest; starts a 24-hour staging revision.
- `PUT /v1/token-bundles/{submission}/{revision}/{artifact}`: exact attachment bytes. The reserved `envelope` object is the exact certified envelope.
- `POST /v1/token-bundles/{submission}/{revision}`: exact envelope and its witness/admission headers; runs ordinary admission and verifies all stored artifacts before committing a receipt.
- `GET /v1/token-bundles/{submission}/{revision}`: authenticated owner status and receipt.
- `GET /v1/token-bundles/{submission}/{revision}/{artifact}`: owner-only token attachment read; verifies current sanitized-event correspondence, refuses revoked/expired/stale bundles, and sends `Cache-Control: no-store`.

The manifest binds envelope, consent, event and attachment digests. PostgreSQL temporarily stores encrypted publication packets so retries publish identical ciphertext after a crash. It clears those packets after object readback succeeds. Finalization requires a persisted parent submission and all ready objects. New revisions cannot overwrite an existing manifest. Committed research attachments have a 30-day retention interval in this initial implementation.

Client cleanup requires an authenticated receipt matching the configured server, tenant, account, submission, revision and manifest. The client writes and syncs its receipt journal before releasing its own immutable Ironwire lease. A background task retries one acknowledged intent at startup and every five minutes. It never deletes original agent session files. Missing or incompatible peers leave cleanup pending.

Withdrawal blocks bundle access through the parent revocation trigger. Existing withdrawal and retention maintenance retry object deletion under the same database locks used for publication. These locks and provider delete calls are not proof that cloud object versions, backups or external copies are erased. Qualify actual backend timeout/versioning semantics, backup restoration and concurrent publication/deletion before production cleanup receipts are enabled.

Raw alternatives are absent from normal transcript/vector APIs. Broader restricted export and index metadata integration remains an acceptance item; this preview exposes only owner-scoped attachment reads.
