# Pilot Allowlist on the Upload-Claim Issuer (Design)

Date: 2026-05-17
Status: DRAFT — implementation gated on operator approval. No code
changes yet.
Owner: Trace Commons / Server lane
Predecessors:
- `docs/trace-commons.md` (envelope contract; section "Upload-claim auth"
  describes the existing EdDSA/Ed25519 issuer flow)
- `crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs`
  (existing issuer MVP; today issues to any caller that presents a valid
  workload token)
- Project memory `project-pilot-scoring-backend` — pilot ships against
  NEAR AI Cloud; first contributor traffic still gates on (a) gate-floor
  recalibration on the hosted model, and (b) Ironclaw client integration.
  This spec adds a third explicit gate: contributor allowlist.

## Goal

Add a contributor allowlist to `trace-commons-upload-claim-issuer` so the
pilot can run "Ironclaw contributors only" without changing the ingest
server's existing tenant / role / grant model. The allowlist is the
issuance boundary; downstream server controls
(`TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS`, central-issuer ABAC,
RLS-pinned tenant scoping) keep operating exactly as today against the
claims the issuer mints.

Out of scope: changes to the ingest server, changes to the envelope
contract, changes to credit-settlement flow, changes to the Ironclaw
client. The issuer is the only binary touched.

## Why the issuer (vs. ingest server or static bearer tokens)

Three places the gate could live; the issuer is the right one:

| Option | Allowlist edit cost | Coverage | Verdict |
|---|---|---|---|
| Issuer allowlist (this spec) | Edit JSON / NEAR contract, no server restart | Catches every fresh claim refresh, so denial latency is bounded by claim TTL | **Chosen** |
| Server tenant-access grants | Admin API per contributor; duplicates the claim flow | Strongest control but operator overhead scales linearly with contributors | Keep enabled as a defense-in-depth layer, not the primary gate |
| Static `TRACE_COMMONS_TENANT_TOKENS` env | Edit env + restart server per change | Hostile to "add a contributor today"; only viable for ≤5 closed-alpha users | Reject |

The issuer also matches the spec's existing reset-and-grow story: today
it's an MVP, this slice lifts it to "production-shaped for pilot".

## Allowlist sources

Issuer takes `--allowlist-source <SPEC>`. Two sources at MVP, one more
deferred:

1. **`file:<path>`** — JSON file on disk. Cheapest, sufficient for closed
   alpha. The issuer re-reads the file every
   `TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS` (default 60), so a
   file edit takes effect within a minute without restart. File schema:
   ```json
   {
     "version": 1,
     "generated_at": "2026-05-17T18:00:00Z",
     "policy_label": "pilot-2026-05",
     "entries": [
       {
         "subject_hash": "sha256:abcd...",
         "tenant_id": "pilot-tenant-zaki",
         "note_label": "closed-alpha-batch-1"
       }
     ]
   }
   ```
   `subject_hash` is `sha256(canonical_subject_string)` where
   `canonical_subject_string` is whatever the issuer's existing workload
   identity verification yields — NEAR account id, EdDSA pubkey hex, or
   issuer-side-issued invite code. The exact subject encoding is the
   issuer's existing convention; we do not change it here. Storing
   subjects as `sha256:` hashes matches the repo's hash-only audit
   convention so the file can be committed publicly without leaking
   contributor identity. `note_label` is operator-facing only, never
   logged or returned.

2. **`null` (no flag)** — allowlist disabled, issuer behaves exactly as
   today (issues to any caller with a valid workload token). This is the
   off-position so the new code does not change current behavior unless
   the operator opts in.

3. **DEFERRED: `near:<account>.<network>:<view_method>`** — on-chain
   allowlist contract. Issuer makes a NEAR view call (no signing, no gas)
   to e.g. `allowlist.tracecommons.near` and checks `subject_hash`
   membership. This is the "open-pilot" growth path: contributors join
   on-chain via a separate flow (sign a participation attestation, etc.)
   and the issuer reflects on-chain state with the same refresh interval.
   Not in MVP scope; the file source has to ship first to unblock closed
   alpha, then we add this source after operating learning. Reserved as a
   `SourceKind::Near { account, method }` variant in the parser so the
   CLI surface doesn't churn.

The source dispatcher lives behind a trait:

```rust
trait AllowlistSource: Send + Sync {
    /// Cheap repeated call; the implementation caches and only refetches
    /// once per refresh interval. Returns the loaded snapshot or the
    /// previously-loaded snapshot on transient fetch error.
    fn snapshot(&self) -> AllowlistSnapshot;
}

struct AllowlistSnapshot {
    policy_label: String,
    generated_at: chrono::DateTime<chrono::Utc>,
    subject_hashes: HashSet<String>,   // canonical "sha256:<64 hex>"
    loaded_at: std::time::Instant,
    source_label: String,              // e.g. "file:/etc/tracecommons/allowlist.json"
}
```

## Issuance flow change

Current flow (preserved):

1. Client posts `POST /v1/trace-upload-claim` with workload identity proof.
2. Issuer validates the workload token (existing path).
3. Issuer signs an EdDSA upload claim with the issuer keyset, embeds
   `tenant_id`, `audience`, `kid`, `exp`, returns it.

New flow with `--allowlist-source` set:

1. (unchanged) Validate workload token.
2. (unchanged) Resolve canonical subject string from workload identity.
3. **NEW**: compute `subject_hash = sha256:<hex(sha256(canonical_subject_string))>`.
4. **NEW**: load current `AllowlistSnapshot`. If `subject_hash` is not in
   `snapshot.subject_hashes`, refuse with HTTP 403 + error class
   `PilotAllowlistNotMatched`. Log one hash-only `tracing::warn` line
   carrying `subject_hash`, `policy_label`, `source_label`. Never log the
   raw subject string. Never echo the missing-match list to the client.
5. (unchanged) Sign and return the claim. The minted claim includes a
   `policy_label` claim ("pilot-2026-05") so server-side audit rows can
   correlate accepted submissions with the allowlist policy active when
   the claim was minted.

Refresh failure handling: if `snapshot()` fails to reload the source
(file deleted, NEAR RPC unreachable), the issuer keeps serving the last
successfully-loaded snapshot until a `--allowlist-max-stale-seconds`
(default 3600) is exceeded. Past that, every issuance refuses with
`PilotAllowlistStale`. Fail-closed beats serving on a stale snapshot
when the operator's intent is "shrink the pilot now".

## Operator-facing readiness

A new `/v1/admin/allowlist-status` endpoint returns counts only, never
identities:

```json
{
  "configured": true,
  "source_label": "file:/etc/tracecommons/allowlist.json",
  "policy_label": "pilot-2026-05",
  "entries": 7,
  "snapshot_age_seconds": 23,
  "denials_last_hour": 0,
  "max_stale_seconds": 3600,
  "stale": false
}
```

`denials_last_hour` is a sliding-window counter held in process memory;
restart resets it. No tenant breakdown — the surface mirrors the existing
`/v1/admin/config-status` convention (booleans, hashes, counts only). If
the allowlist is unconfigured the field set degrades to
`{ "configured": false }` only.

The admin endpoint is gated behind the issuer's existing operator bearer
token (whatever that is today; the issuer is a separate process from
ingest and has its own auth surface — confirm at implementation time).

## Failure modes and refusal vocabulary

New error class labels, added to whatever the issuer's existing error
taxonomy is:

| Label | When | HTTP |
|---|---|---|
| `PilotAllowlistNotMatched` | `subject_hash` not in current snapshot | 403 |
| `PilotAllowlistStale` | Snapshot exceeded `max_stale_seconds` | 503 |
| `PilotAllowlistSourceMissing` | `--allowlist-source` references a path/account that has never loaded | 500 (startup fails closed; this only fires if the operator pointed at a moving target post-startup) |
| `PilotAllowlistMalformed` | JSON parse failure or schema mismatch on reload | 503 (keep serving stale until max-stale; do not adopt malformed file) |

All four propagate as `tracing::warn` with `error_class` set to the
label, `subject_hash` (where applicable), and `source_label`. Never the
raw file contents, never the raw subject string, never the workload
token.

## CLI surface (additions only)

```
--allowlist-source <SPEC>
    Allowlist source for pilot gating. Format:
      file:<absolute_path>         JSON allowlist on disk.
      (omit)                        Allowlist disabled — issue to any
                                    caller with valid workload token.
    Reserved for a later slice:
      near:<account>:<view_method> NEAR view-call to a contract allowlist.

--allowlist-refresh-interval-seconds <N>   (default 60)
--allowlist-max-stale-seconds <N>          (default 3600)
```

Env-var equivalents:
- `TRACE_COMMONS_ALLOWLIST_SOURCE`
- `TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS`
- `TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS`

## Test plan

- **Unit (pure)**: `AllowlistSnapshot` membership lookup; JSON
  deserializer including (a) unknown fields tolerated, (b) duplicate
  subject_hashes deduped, (c) entries with malformed hashes rejected at
  parse time.
- **Unit (file source)**: refresh interval honored — repeated `snapshot()`
  inside the interval returns cached, after the interval reloads; file
  deletion mid-run keeps the cached snapshot until max-stale; corrupt
  file mid-run keeps the cached snapshot.
- **Integration (issuer)**: post a claim request for a known-allowed
  subject → 200 with policy_label embedded; same request after removing
  the subject from the file + waiting past refresh → 403
  `PilotAllowlistNotMatched`; deleting the file + waiting past max-stale
  → 503 `PilotAllowlistStale`.
- **Admin endpoint contract test**: `/v1/admin/allowlist-status` returns
  counts only and never any field whose value contains a non-hashed
  identity, validated against a fixture allowlist with deliberately
  identifiable `note_label` strings.

## Migration / rollout

1. Land the issuer changes off by default
   (`--allowlist-source` omitted → behavior is identical to today's MVP).
2. Operator stages an allowlist JSON with the closed-alpha set, signs off
   that `/v1/admin/allowlist-status` reports the expected entries count
   and zero denials.
3. Operator restarts the issuer with `--allowlist-source=file:...`.
4. First denial-on-purpose smoke (a known-not-allowlisted test subject
   posts a claim request, expects 403 + `PilotAllowlistNotMatched`).
5. Pilot opens to allowlisted contributors. Adding contributors is a
   file edit; refresh interval bounds how long they wait.

Rollback: remove `--allowlist-source` and restart; behavior reverts to
today's MVP. No on-disk state outside the JSON file itself.

## Open questions for the owner

1. **Canonical subject string** — what does the issuer's current
   workload-identity verification produce? NEAR account id?
   issuer-issued invite code? An EdDSA pubkey hex? The spec assumes
   whatever it is gets `sha256:`-hashed verbatim. Confirm at
   implementation time to lock the encoding before the first allowlist
   file ships.
2. **Operator bearer for `/v1/admin/allowlist-status`** — confirm the
   issuer has an existing operator auth surface to mount this on; if
   not, this endpoint either needs to defer or we add a tiny
   `TRACE_COMMONS_ISSUER_ADMIN_BEARER_TOKEN` env at the same time.
3. **`policy_label` in the minted claim** — does the existing claim
   schema have an extensions slot we can add this to without breaking the
   client's JWT validator? If yes, embed; if no, defer the embedding to
   a follow-up and just log it issuer-side for now.

These don't block drafting the implementation plan against this spec;
they block the first `cargo check` on it.
