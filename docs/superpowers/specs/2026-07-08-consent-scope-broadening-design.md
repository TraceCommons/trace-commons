# Consent-Scope Broadening for Device-Key Claims — Design

Date: 2026-07-08
Status: Implemented
Depends on: PR #154 (contributor uploader CLI), instance-vouched enrollment (PR #150)

## Purpose

Device-key upload claims are currently hardcoded to consent scopes
`[debugging_evaluation, public_attribution]` and allowed uses
`[debugging, evaluation, aggregate_analytics]`
(`trace_upload_claim_issuer.rs` `device_key_allowed_consent_scopes()` /
`device_key_allowed_uses()`), so no trace submitted by the contributor CLI
can carry training-usable consent. This slice makes the enrollment-stored
instance policy the ceiling for device-key claims and threads contributor
scope choice end-to-end through the CLI.

Key discovery grounding the design: the storage and enforcement machinery
already exist. The allowlist's `InstancePolicyTemplate` carries per-instance
`allowed_consent_scopes` / `allowed_uses`; enrollment hands them to the DB
via `InstanceUserProvision`, which provisions a tenant-access-grant row for
the device principal (`device:{tenant_id}:{device_key_id}`); the issuer
already has grant lookup and scope-intersection code on the workload path;
ingest already enforces claim scopes.

Implementation-time correction (2026-07-08 research): the grant writer
(`upsert_onboarding_device_tenant_access_grant`, db/postgres.rs) currently
IGNORES the provision's template scopes and hardcodes pilot defaults
(`["debugging_evaluation","public_attribution"]`). So the slice has two
server gaps, not one: (a) the grant writer must persist the template scopes
(falling back to the pilot defaults when the template omits them), and
(b) the device-key claim path must derive its ceiling from the grant.

## Decisions (settled during brainstorming)

| Question | Decision |
|---|---|
| Policy model | Instance policy template is the ceiling; claim gets `intersect(requested, ceiling)`. Contributor choice is the requested set. |
| Invite-code device keys | Unchanged: hardcoded floor stays. Only instance-enrolled keys (grant row present) get broader scopes. |
| Slice scope | End-to-end: issuer + claim response + CLI (`login --scopes`, claim requests, envelope stamping) in one slice. |
| Storage | Approach A: read the enrollment-provisioned tenant access grant at claim time. No schema migration. |
| Attestation format | Unchanged. No per-user scopes in the signed attestation. |

## Server design

### Scope resolution in `issue_claim_for_device_key` (and the device-JWT variant)

After device-key verification succeeds:

1. Determine the ceiling:
   - If a tenant-access-grant DB is configured AND a grant row exists for
     `grant_principal_ref` = `principal_storage_ref("device:{tenant_id}:{device_key_id}")`,
     the ceiling is that grant's stored `allowed_consent_scopes` /
     `allowed_uses`.
   - Otherwise (no grant DB configured, or no row — e.g. invite-path keys):
     the ceiling is exactly today's hardcoded
     `device_key_allowed_consent_scopes()` / `device_key_allowed_uses()`.
     This branch must be byte-for-byte behavior-compatible with today.
2. Validate the request: any `consent_scopes` / `allowed_uses` entry that
   does not parse as a known enum value is a 400 with label
   `"invalid consent scope"` (or `"invalid allowed use"`).
3. Intersect: issued claim carries
   `intersect(requested_scopes, ceiling_scopes)` and
   `intersect(requested_uses, ceiling_uses)`. An empty requested list keeps
   today's behavior of granting the full (hardcoded-floor) ceiling only in
   the no-grant branch; in the grant branch an empty request grants the
   full grant ceiling. (Rationale: pre-this-slice CLIs send
   `["debugging_evaluation"]`, so nothing silently broadens for them.)
4. Empty intersection on `consent_scopes` is a 403 with label
   `"consent scopes not permitted"` — never an empty-scope claim.
5. `public_attribution` remains always permitted on the device path (added
   to every ceiling) so community-profile flows do not regress.

Hash-only logging: denial logs carry scope enum labels and the
device_key_id only — both are non-identifying.

### Wire change (additive)

`TraceUploadClaimResponse` gains:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub consent_scopes: Vec<ConsentScope>,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub allowed_uses: Vec<TraceAllowedUse>,
```

echoing exactly what the issued JWT carries. Old clients ignore the fields.
The request format is unchanged (`consent_scopes` / `allowed_uses` already
exist there).

## CLI design

- `login` gains `--scopes <csv>` of ConsentScope wire names, validated
  client-side against the known set; `public_attribution` is only included
  when explicitly listed. The chosen set is stored in the existing
  `ContributorConfig.consent_scopes` field (which becomes load-bearing) and
  echoed in the login consent line.
- **Interactive consent prompt (the natural flow).** When `--scopes` is
  not given and stdin is a TTY, `login` prompts in plain language after
  successful enrollment, one line per optional scope:

  ```
  How may your submitted traces be used? (you can revoke submitted traces later)
    Debugging and evaluation                 [always on]
    Benchmark generation                     [y/N]
    Ranking-model training                   [y/N]
    Model training                           [y/N]
    Public attribution of your handle        [y/N]
  ```

  Defaults are conservative (everything optional defaults to No);
  `debugging_evaluation` is always included. Answers map to ConsentScope
  values and are stored/echoed identically to `--scopes`. When stdin is
  not a TTY and `--scopes` is absent, the default is `debugging_evaluation`
  only (no prompt, no hang in scripts). The prompt logic is a pure helper
  (answers in, scopes out) so it is testable without a TTY.
- Claim requests send the configured scopes plus the matching allowed-uses
  mapping: `debugging_evaluation -> [debugging, evaluation]`,
  `benchmark_only -> [benchmark_generation]`,
  `ranking_training -> [ranking_model_training]`,
  `model_training -> [model_training]`,
  `public_attribution -> []` (no trace-use), and `aggregate_analytics` is
  always requested (matches today's device-key floor).
- Envelope stamping: `consent.scopes` (and the derived `trace_card`
  fields) come from the claim **response's** granted set — never directly
  from config — so an envelope can never claim more than the server
  granted. If the response omits the new fields (older issuer), fall back
  to stamping the requested set; ingest enforcement still caps it.
- A 403 scope refusal at claim time maps to
  `SubmitOutcome::Refused { "scopes-not-permitted" }` with a printed hint
  to re-run `login --scopes` with a narrower selection.
- README consent-model section: replace the "v1 cap" language with the
  instance-template-ceiling description.

## Scope visibility (added)

Contributors must be able to see, server-side, what consent each submitted
trace actually carries:

- `TraceSubmissionStatusUpdate` gains an additive field
  `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub consent_scopes: Vec<ConsentScope>`
  populated from the stored envelope's consent block by the
  submission-status handler.
- `trace-commons-contributor status` gains a SCOPES column rendering the
  wire names comma-joined (empty cell when the field is absent from an
  older server).
- This is contributor-authenticated read-back only. Consent settings never
  appear on the public community-profile surface.

Retroactive consent updates (upgrading old traces to broader scopes, or
partial downgrades short of full revocation) are explicitly deferred to the
next slice: they require a consent-change endpoint with claim-backed proof,
hash-only audit, and (for downgrades) the invalidation-propagation path.

## Error handling summary

| Condition | Layer | Result |
|---|---|---|
| Unknown scope string in request | issuer | existing 400 `"invalid upload claim request"` (serde already rejects unknown enum values at parse time; no new label) |
| Empty scope intersection | issuer | 403 `"consent scopes not permitted"` |
| Scope refusal at submit | CLI | `Refused { "scopes-not-permitted" }` + re-login hint, batch continues |
| No grant row / no grant DB | issuer | hardcoded floor, today's behavior exactly |

## Testing

Issuer:
- Grant present: narrower request narrows the claim; broader request is
  clipped to the grant ceiling; JWT `allowed_consent_scopes` equals the
  response fields.
- No grant / no grant DB: regression pin asserting today's exact hardcoded
  behavior.
- Empty intersection → 403 with the exact label; unknown scope → 400.
- `public_attribution` always granted on the device path.

CLI:
- `--scopes` parsing and validation (unknown name rejected with the set of
  valid names in the message).
- Interactive-prompt helper: answer combinations map to the right scope
  sets; all-No yields `[debugging_evaluation]`; non-TTY default path
  yields `[debugging_evaluation]` without prompting.
- Envelope stamps the response-granted set (stub issuer returns a narrowed
  set); fallback stamping when the response lacks the fields.
- Scope-refusal outcome mapping.

Scope visibility:
- Submission-status handler returns the stored envelope's consent scopes;
  CLI status renders the SCOPES column and tolerates the field's absence.

E2E (extends `e2e_enroll_and_submit`):
- Allowlist `policy_template` grants `model_training` (+ floor);
  `login --scopes debugging_evaluation,model_training`; submit; assert the
  stub-received envelope's `consent.scopes` equals the granted set. The
  in-memory Database gains a working tenant-access-grant lookup (it
  already receives the provision input at enrollment).

## Out of scope

- Retroactive consent updates on already-submitted traces (upgrade or
  partial downgrade) — next slice, per 2026-07-08 decision.
- Invite-path scope broadening.
- Per-user scopes inside the signed enrollment attestation.
- Ingest-side enforcement changes (claim-scope enforcement already
  exists). The only ingest edit in this slice is the additive
  `consent_scopes` field on the submission-status response.
- Retention-policy changes and credit changes.
- Schema migrations (none needed; grant columns exist and are written).
