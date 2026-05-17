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
   `subject_hash` is `sha256:hex(sha256("invite:" + invite_code))`. The
   pilot uses operator-issued **invite codes** as the canonical subject —
   no NEAR account lookup, no EdDSA pubkey ceremony. The `"invite:"`
   prefix namespaces the hash so a later subject type (e.g. NEAR account
   ids in the open-pilot phase) cannot collide accidentally. Storing
   subjects as `sha256:` hashes matches the repo's hash-only audit
   convention so the file can be committed publicly without leaking
   contributor identity. `note_label` is operator-facing only, never
   logged or returned.

   The invite code reaches the issuer via a new optional `invite_code`
   field on the existing `WorkloadClaims` struct
   (`crates/trace-commons-server/src/trace_upload_claim_issuer.rs:333`).
   When `--allowlist-source` is set the field becomes required and a
   missing value yields HTTP 400 `PilotAllowlistInviteCodeMissing`.
   The Ironclaw client and the dev workload-token signer both need to
   start populating it before any allowlisted contributor's first
   refresh succeeds. This is the only client-visible protocol change
   in the slice.

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
   the claim was minted. This is one new `Option<String>` field on the
   existing `UploadClaimClaims` serde struct
   (`trace_upload_claim_issuer.rs:354`); the JSON-Web-Token validator on
   the ingest side (jsonwebtoken-rs) silently ignores unknown claims by
   default, so this is non-breaking for clients that haven't been
   updated yet. The ingest server can start reading it in a follow-up
   slice once it's flowing.

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

The issuer has no operator-bearer surface today. The router exposes
`/health`, `/.well-known/trace-commons-ed25519-keyset.json`, and
`POST /v1/trace-upload-claim` — none of them admin-gated. Two ways to
mount the new admin endpoint; recommend the second:

1. **New bearer-auth env**: `TRACE_COMMONS_ISSUER_ADMIN_BEARER_TOKEN`,
   middleware that rejects requests without `Authorization: Bearer <token>`
   matching the env. Standard pattern, but adds a long-lived secret to
   the operator's footprint.
2. **Separate admin bind** (recommended): add
   `TRACE_COMMONS_ISSUER_ADMIN_BIND` (default `127.0.0.1:0` =
   disabled). When set, spin a second axum server on that address with
   only `/v1/admin/*` routes. No bearer needed; operator hits it via SSH
   tunnel or direct localhost curl on the issuer host. Matches how many
   ops tools expose hostmetric endpoints. Fail-closed: if the env is
   set but the bind fails, the whole issuer refuses to start with
   `PilotAllowlistAdminBindFailed`.

Either pattern is mechanical to implement. The localhost-only bind has
fewer secrets to rotate; the bearer pattern is one more env. Pick at
implementation time; the spec defaults to option 2 unless the operator
objects.

## Failure modes and refusal vocabulary

New error class labels, added to whatever the issuer's existing error
taxonomy is:

| Label | When | HTTP |
|---|---|---|
| `PilotAllowlistNotMatched` | `subject_hash` not in current snapshot | 403 |
| `PilotAllowlistStale` | Snapshot exceeded `max_stale_seconds` | 503 |
| `PilotAllowlistSourceMissing` | `--allowlist-source` references a path/account that has never loaded | 500 (startup fails closed; this only fires if the operator pointed at a moving target post-startup) |
| `PilotAllowlistMalformed` | JSON parse failure or schema mismatch on reload | 503 (keep serving stale until max-stale; do not adopt malformed file) |
| `PilotAllowlistInviteCodeMissing` | Workload claims lack the `invite_code` field when allowlist is configured | 400 |
| `PilotAllowlistAdminBindFailed` | `TRACE_COMMONS_ISSUER_ADMIN_BIND` is set but the bind fails at startup | (startup-only; process exits non-zero) |

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
- `TRACE_COMMONS_ISSUER_ADMIN_BIND` (e.g. `127.0.0.1:8081`; when set,
  mounts `/v1/admin/allowlist-status` on this address. Unset = disabled.)

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

All three of the original draft questions resolved 2026-05-17:

1. **Canonical subject string** — RESOLVED. Invite codes. No crypto,
   no NEAR account lookup. The issuer reads a new `invite_code` field
   off the existing `WorkloadClaims` struct, the allowlist file stores
   `sha256:hex(sha256("invite:" + invite_code))`, the operator hands
   contributors invite codes out of band (form, DM, whatever fits the
   pilot's recruiting flow). Inline in the relevant sections above.
2. **Operator bearer for the admin endpoint** — RESOLVED.
   The issuer has no admin auth surface today. Spec recommends a
   separate localhost-only admin bind
   (`TRACE_COMMONS_ISSUER_ADMIN_BIND`) over a long-lived bearer token,
   so there's no new secret to rotate. Either pattern works; localhost
   bind is the default unless the operator objects at implementation
   time.
3. **`policy_label` in the minted claim** — RESOLVED. The existing
   `UploadClaimClaims` is a serde struct
   (`trace_upload_claim_issuer.rs:354`); `policy_label: Option<String>`
   is a one-line additive field, and jsonwebtoken-rs ignores unknown
   claims on the validator side, so this is non-breaking for clients
   that haven't been updated. The ingest server starts reading it in a
   follow-up slice.

Nothing else parked. Ready to draft the implementation plan against
this spec on operator approval.
