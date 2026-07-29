# Server-signed score attestations — implementation report

Branch: `worktree-agent-a4558723d931baf70` (worktree reset to
`origin/worktree-devfolio-feedback-fixes`, HEAD `d26dd97`)
Worktree: `/Users/zakimanian/code/trace-commons-server/.claude/worktrees/agent-a4558723d931baf70`
Spec: `docs/superpowers/specs/2026-07-29-score-attestation-design.md`

## Scope delivered (server-side only, per instructions)

1. **New module** `crates/trace-commons-server/src/trace_score_attestation.rs`
   - `AttestationConfig::from_env()` reads
     `TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM`,
     `TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM`,
     `TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID` (all three, per spec),
     plus an optional `TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS`
     (default 24h). Returns `Ok(None)` only when ALL three required envs are
     absent (attestations disabled, endpoint fails closed at request time);
     returns `Err` on any partial/malformed configuration — a half-configured
     signer is an operator error, not a silent disable.
   - `AttestationSigningState::build()` validates the PEM material via the
     SAME `validate_eddsa_private_key_pem` / `validate_eddsa_public_key_pem`
     helpers the upload-claim issuer uses (made `pub(crate)` in
     `trace_upload_claim_issuer.rs` rather than duplicated).
   - `sign_score_attestation()` builds `ScoreAttestationClaims` (schema
     version, tenant_id, auth_principal_ref, submissions, issued_at,
     expires_at, nonce — exact field order from the spec's JSON example) and
     signs a compact JWS via `jsonwebtoken::encode` with `Algorithm::EdDSA`
     and `kid` in the header. `expires_at` is unconditionally
     `issued_at + ttl_seconds`; there is no code path that omits it.
   - `keyset_json()` returns `{ "keys": [{ "kid", "public_key_pem" }] }`,
     matching `trace_upload_claim_issuer::keyset_handler`'s shape exactly.
   - `ATTESTATION_SIGNING_KEY_UNCONFIGURED = "attestation_signing_key_unconfigured"`
     — the fail-closed label used by both new endpoints.
   - 4 unit tests: `from_env` None-when-unconfigured, `from_env` errors on
     partial config, sign+verify round-trip (asserts kid, alg, schema
     version, expires_at derivation, nonce), and a negative test that a
     signature does NOT verify against a different keyset entry.

2. **DB layer**
   - `Database::list_own_gate_decision_scores(tenant_id, auth_principal_ref,
     limit)` added to `db/mod.rs` (default empty impl, mirroring the other
     gate-driver-pool methods) and implemented in `db/postgres.rs`. The
     query joins `trace_gate_decisions` to `trace_submissions` and filters
     on `s.tenant_id = $1 AND s.auth_principal_ref = $2`, keeping the latest
     decision per submission (`DISTINCT ON` + `decided_at DESC, decision_id
     DESC`, same tiebreaker convention as `list_scores_by_submission_ids`).
     Both identity values are supplied by ingest's own authenticated-request
     code, never by a client-suppliable id/filter, so scoping is enforced
     both in the query text (defense in depth) and structurally (the
     endpoint has no parameter to carry an override).
   - Reuses the existing `TraceScoreBySubmissionRow` struct — no new wire
     type needed on the storage side.
   - Added an in-memory analogue on `PerplexityDriverTestDb` (the ingest
     binary's test double) so the handler tests don't need a live Postgres.

3. **Ingest binary** (`bin/trace-commons-ingest.rs`)
   - `AppState.attestation_signing: Option<Arc<AttestationSigningState>>`,
     built once at startup from `AttestationConfig::from_env()` (startup
     fails hard on malformed config, exactly like the issuer's key loading).
   - `GET /v1/contributors/me/score-attestation` —
     `score_attestation_handler`. Resolves `tenant_id` / `auth_principal_ref`
     ONLY from `authenticate_ctx_with_tenant_access_grant(...)` (the same
     auth path `credit_handler` / `submission_status_handler` use). **The
     handler signature is `(State<Arc<AppState>>, HeaderMap)` — no body
     extractor, no query extractor.** There is structurally no parameter
     through which a caller could supply a principal; this is the
     non-negotiable from the spec, enforced by the absence of any such
     extractor rather than by a runtime check that could be forgotten later.
     503s with `attestation_signing_key_unconfigured` when the signing key
     isn't configured, and separately 503s when no DB mirror /
     gate-driver pool is configured (mirroring `scores_by_submission_handler`'s
     fail-closed posture). On success, emits one hash-only
     `append_control_plane_read_audit` row (surface `score_attestation`,
     count only — no submission ids, no principal).
   - `GET /.well-known/trace-commons-attestation-keyset.json` —
     `attestation_keyset_handler`, unauthenticated (public key material
     only), 503s with the same missing-control label when unconfigured.
   - Both routes registered on the main router next to the existing
     `/v1/contributors/me/*` routes.
   - Two new handler tests in
     `trace_commons_ingest_internal/tests.rs`:
     - `score_attestation_handler_signs_only_the_callers_own_scores_and_fails_closed`
       — pins the 503 when unconfigured, the 503 when no DB mirror, a
       successful attestation that verifies against the published key and
       contains only the caller's own submission, AND (the core
       non-negotiable) that a second contributor (`token-a-2`) authenticated
       with their own token gets THEIR OWN scores back — never the first
       contributor's — even though both submissions live in the same
       tenant. Comment on the test explains the property is structural
       (the handler has no parameter to forge through).
     - `attestation_keyset_handler_fails_closed_then_publishes_the_key`.

## Non-negotiables checked against the spec

- Principal resolved from caller auth only, never a request parameter:
  satisfied structurally (no body/query extractor on the handler) and
  pinned by the two-contributor test above.
- Fail-closed with `attestation_signing_key_unconfigured` (503), never an
  unsigned document: both endpoints check `state.attestation_signing` first
  and return before touching the DB or the signer.
- `expires_at` mandatory: baked into `sign_score_attestation`, no optional
  path.
- Hash-only logging: the only `tracing`/audit output on this path is the
  existing `append_control_plane_read_audit` (surface label + count),
  identical to the pre-existing devfolio route's posture. No key material,
  raw principal, or submission ids appear in any log/audit string I added.
- No new dependencies: only `ring`/`jsonwebtoken`/`serde`/`uuid`/`chrono`,
  all already in the workspace and already used by
  `trace_upload_claim_issuer.rs`.

## Underspecified points / judgment calls (flagging, not deviating)

- The spec's env var list has exactly three keys (no `_FILE` sibling like
  the issuer's `..._SIGNING_PRIVATE_KEY_FILE` pattern). I followed the spec
  literally rather than adding an unrequested `_FILE` variant.
- The spec doesn't specify the attestation endpoint's HTTP method/path
  exactly — I used `GET /v1/contributors/me/score-attestation`, matching
  the existing `/v1/contributors/me/*` family and the fact that this is a
  read with no body needed (which also makes the "no parameter" invariant
  trivially checkable). If a POST with an empty body is preferred for
  consistency with `submission_status_handler`, that's a one-line change to
  the route/handler.
- The spec doesn't say what the keyset endpoint should do when signing is
  unconfigured (empty `keys: []` vs. 503). I chose 503 with the same
  missing-control label, on the theory that an empty array could be
  misread by an automated collector as "no keys published yet, retry
  later" rather than "misconfigured deployment."
- `--generate-attestation-keypair` subcommand (spec's "Key material and
  configuration" section) is NOT implemented — the spec assigns it to the
  ingest binary's CLI surface, but the task instructions scoped this task to
  "config, key loading, keyset route, the attestation endpoint, and the DB
  query" and explicitly excluded new CLI surface to avoid colliding with
  parallel work on the same branch. `AttestationSigningState::build()`
  reuses the same validation as the issuer's existing
  `generate_upload_claim_keypair()` (which already produces
  spec-compliant PKCS#8 v2 / SPKI material), so wiring a thin
  `--generate-attestation-keypair` subcommand later is a small, low-risk
  follow-up that just calls that existing function and prints the PEM
  blocks — flagging this as an explicit gap rather than silently leaving it
  unmentioned.
- The contributor CLI `attest` subcommand is out of scope per instructions
  (parallel work) and was not touched.

## Commands run (all against this worktree)

```
git fetch origin
git reset --hard origin/worktree-devfolio-feedback-fixes
git log --oneline -4   # confirmed d26dd97 at HEAD

cargo fmt -p trace-commons-server        # normalized formatting after edits
cargo fmt --check                        # clean, no diff

RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
  # Finished `dev` profile ... in 11-16s (clean, multiple re-runs)

RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
  # Finished `dev` profile ... in 35.72s (clean)

RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
  # builds lib + all 9 bins + all integration test binaries, clean

cargo clippy -p trace-commons-server -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
  # Finished `dev` profile ... in 12.27s (clean)

cargo test -p trace-commons-server --lib
  # test result: ok. 171 passed; 0 failed

cargo test -p trace-commons-server --bin trace-commons-ingest
  # test result: ok. 839 passed; 0 failed (includes the 2 new tests)

cargo test -p trace-commons-contributor
  # test result: ok. 97 + 1 + 4 passed; 0 failed (untouched by this change)

cargo test -p trace-commons-server --test trace_corpus_storage_contract
  # test result: ok. 15 passed; 0 failed
```

Postgres-backed tests (`trace_corpus_pg_store`, `trace_corpus_pg_rls`) were
not run — no live Postgres was reachable in this environment (connection to
`localhost:5432` refused/role missing), and per repo convention these are
not part of the CI gate. The new `list_own_gate_decision_scores` SQL was
written by close structural analogy to
`list_contributor_cap_signals`/`list_scores_by_submission_ids` (same pool,
same `DISTINCT ON` + tiebreaker convention) but has NOT been exercised
against a live database in this session — worth a real-DB smoke pass before
this ships, consistent with the "fixtures may match the bug" caution in
project memory.

## One incidental issue caught and fixed

Partway through, a PostToolUse formatting hook ran a full-workspace
reformat using a rustfmt configuration that disagreed with the repo's
already-committed style, producing a ~5,700-line spurious diff across
`trace_commons_ingest_internal/tests.rs` and an unrelated one-line change to
`db/trace_corpus_common.rs`. I caught this by running `cargo fmt --check`
against the clean base branch (0 diff) vs. after my edits (many diffs,
almost none of which were in code I'd touched), then ran `cargo fmt -p
trace-commons-server` myself, which reconverged everything to a minimal,
correct diff and reverted the unrelated file. Final `cargo fmt --check` is
clean.

## Files touched

- `crates/trace-commons-server/src/trace_score_attestation.rs` (new)
- `crates/trace-commons-server/src/lib.rs` (module registration)
- `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`
  (`validate_eddsa_private_key_pem`, `validate_eddsa_public_key_pem`,
  `required_env`, `optional_env` made `pub(crate)` for reuse; no behavior
  change)
- `crates/trace-commons-server/src/db/mod.rs` (new trait method + default)
- `crates/trace-commons-server/src/db/postgres.rs` (new Postgres impl)
- `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  (`AppState` field, startup wiring, two new handlers, two new routes)
- `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`
  (test-double method, two new handler tests, one test-state helper)
