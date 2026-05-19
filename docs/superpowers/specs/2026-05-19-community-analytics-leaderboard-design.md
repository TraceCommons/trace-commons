# Community Analytics & Leaderboard — Design

Date: 2026-05-19
Status: Draft (pre-implementation)

Owner: Trace Commons / Community surface (new lane)

## Decision frame

The pilot operator dashboard
([`docs/operator/pilot-dashboard.md`](../../operator/pilot-dashboard.md))
covers internal-only visibility against pseudonymous identifiers and
ships now off Cloud SQL via Grafana. This spec is the production
**community-facing** surface: a public leaderboard plus aggregate
analytics that contributors can be proud to be on without
compromising the corpus's privacy posture.

Two surfaces fight each other:

- The corpus is **redacted, hash-only, fail-closed**. Audit rows
  carry `principal_sha256:...`; envelopes carry no raw bodies; even
  tenant ids are hashed in operator-secret material.
- A leaderboard wants **public attribution and meaningful display
  names** so contributors can show "I'm in the top 20."

The bridge is an **explicit, separate opt-in** that maps a
contributor's pseudonymous `principal_ref` to a **self-declared
display handle** they consent to publish. Without that opt-in, the
contributor's submissions still count toward credits and toward
private aggregates, but they never appear by name on any public
surface.

## Goal

A read-only public surface that lets the Trace Commons community see:

1. Who's contributing (by self-declared handle only) and how much.
2. What the corpus looks like in aggregate (volume over time,
   accept rate, gate-decision distribution, novelty distribution),
   subject to k-anonymity and noise.
3. Their own contributor page (handle, credits, badges, opt-in
   status).

…without ever exposing:

- Raw envelope content or per-trace tool sequences linked to a
  handle.
- Cross-tenant joins that violate operator-scoped consent.
- Identity material that isn't self-declared.
- Anything about contributors who have not opted in to public
  display.

## Non-goals

- Rewards distribution or payouts. The credit ledger already settles
  separately; this surface only **displays** credit totals.
- Real-name identity verification. Display handles are
  self-declared; the spec does not require Github/Twitter/etc.
  linkage (a future slice can add verification badges, out of
  scope here).
- Per-trace details on the public surface. The pilot dashboard
  covers per-trace reviewer needs; the public surface aggregates.
- Cross-tenant queries that bypass the existing analytics min-cell
  / noise guards (`TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT`,
  `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE`,
  `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_PRIVACY_ACCOUNTING`). The
  surface piggybacks on those gates rather than inventing new ones.
- Contributor-to-contributor messaging, social graph, comments.
  Public surface is leaderboard + analytics, not a forum.

## Current shape

There is nothing today. The pilot has:

- `trace_credit_ledger` rows scoped to `auth_principal_ref`
  (pseudonymous).
- `trace_submissions` with `auth_principal_ref` + accept/reject
  outcome.
- `trace_gate_decisions` with score distributions.
- The analytics-min-cell guard already enforced in the binary for
  any aggregate surface.

What's missing:

- A consent surface for "publish my handle and stats publicly."
- A handle registry distinct from `principal_ref`.
- A public read-side API + caching surface.
- A spam / abuse mitigation layer for handles.

## Threat model

Attackers we design against:

- **Handle squatters.** Reserve common handles or rotate aggressively
  to claim a position. Mitigation: reserved-name list, rate-limit
  profile changes, audit trail on every profile mutation.
- **Leaderboard farmers.** Submit thousands of low-quality traces to
  climb the rank. Mitigation: leaderboard primary metric is
  **novelty-weighted credit**, not raw count; existing gate floors
  reject low-novelty before they hit the ledger. Display rejection
  rate alongside accept rate.
- **De-anonymization via aggregate joins.** Combine "submissions
  per hour" with timing data from a contributor's own machine to
  re-identify the pseudonym. Mitigation: time bucketing (hour or
  day, never raw timestamp), min-cell on every aggregate, noise on
  long-tail buckets, no per-tool-kind public attribution.
- **Public-display abuse.** Profanity, impersonation, slurs in
  handles. Mitigation: handle validation (alphanumeric +
  hyphen/underscore, length cap, reserved-name list), abuse
  takedown flow with operator audit.
- **Stale-opt-in misuse.** Contributor opts in, regrets it,
  withdraws — but cached pages still show them. Mitigation:
  withdrawal is `UPDATE … SET withdrawn_at = NOW()`, the read API
  filters on `public_since IS NOT NULL AND withdrawn_at IS NULL`,
  the cache TTL is short (≤15 min) and a withdrawal forces a
  targeted cache invalidate.
- **Tenant-leak via display.** A handle's stats reveal which tenant
  they belong to (one-tenant tells one-handle when the cohort is
  small). Mitigation: in pilot the surface is feature-flagged off
  until ≥2 tenants have ≥`min_cell_count` contributors; until
  then the operator dashboard is the only surface.

## Data model

New tables (all RLS-enforced like the rest of the corpus). All
column names use the existing repo conventions (snake_case,
TIMESTAMPTZ, principal-ref naming).

### `trace_contributor_profiles`

One row per (tenant, principal) that has opted in. Withdrawal is a
soft-delete (`withdrawn_at`) so the audit trail survives.

```sql
CREATE TABLE trace_contributor_profiles (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    principal_ref TEXT NOT NULL,          -- principal_sha256:... pseudonym
    display_handle TEXT NOT NULL,         -- self-declared, validated, unique per tenant
    handle_normalized TEXT NOT NULL,      -- lower-case + nfc for uniqueness check
    bio TEXT,                             -- 280-char optional
    public_since TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    withdrawn_at TIMESTAMPTZ,             -- NULL = currently public
    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_count INTEGER NOT NULL DEFAULT 0,  -- rate-limit counter
    PRIMARY KEY (tenant_id, principal_ref),
    UNIQUE (tenant_id, handle_normalized)
);
```

Hash-only audit is satisfied because the `display_handle` IS the
self-declared public material — it's no longer operator-secret once
the contributor has opted in. `principal_ref` stays as it is
everywhere else; the handle is a controlled join key.

### `trace_contributor_profile_audit`

Append-only mirror of every profile mutation, including failed
updates that hit rate-limits. Feeds the abuse takedown flow and the
existing `trace_audit_events` chain via the standard mechanism.

```sql
CREATE TABLE trace_contributor_profile_audit (
    tenant_id TEXT NOT NULL,
    audit_sequence BIGSERIAL NOT NULL,
    principal_ref TEXT NOT NULL,
    action TEXT NOT NULL,           -- 'opt_in' | 'update' | 'withdraw' | 'rejected'
    handle_normalized TEXT,
    reason TEXT,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, audit_sequence)
);
```

### `trace_leaderboard_snapshots`

Materialised aggregates. Computed by a worker on a schedule (default
15 min) so the public read path is fast and consistent. The
write-side is the only place that performs the min-cell + noise
checks; the read-side just reads.

```sql
CREATE TABLE trace_leaderboard_snapshots (
    snapshot_id UUID PRIMARY KEY,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    window_label TEXT NOT NULL,             -- '7d' | '30d' | 'all'
    metric TEXT NOT NULL,                   -- 'novelty_credit' | 'accepted_count'
    contents_jsonb JSONB NOT NULL,          -- pre-rendered leaderboard page
    contents_sha256 TEXT NOT NULL,
    min_cell_count INTEGER NOT NULL,        -- captures the guard at compute time
    noise_seed_hash TEXT NOT NULL           -- so an audit can re-verify
);
```

Pre-rendered JSON keeps the public read path single-query and lets
us invalidate just by inserting a new snapshot.

## API surface

All endpoints are unauthenticated and read-only. They mount under a
new `/v1/community/...` path so the existing `/v1/admin/*` and
`/v1/traces/*` surfaces stay operator-scoped.

| Method | Path | Returns |
|---|---|---|
| GET | `/v1/community/leaderboard?metric=novelty_credit&window=7d&limit=50` | Top contributors by metric; returns the latest snapshot of (`display_handle`, `score`, `accepted_count`, `accept_rate`) |
| GET | `/v1/community/contributors/{display_handle}` | Public profile: handle, bio, public_since, totals, rolling window stats — only if the contributor is currently public |
| GET | `/v1/community/analytics/summary?window=7d` | Corpus-wide aggregates: submissions, accept rate, gate-decision distribution, novelty histogram, all subject to min-cell + noise |
| GET | `/v1/community/analytics/snapshots` | List recent snapshot ids + timestamps for transparency |

Write-side (contributor-authenticated via the existing
upload-claim-token path, gated by a new consent scope —
see "Consent contract" below):

| Method | Path | Action |
|---|---|---|
| PUT | `/v1/community/profile` | Opt in or update (handle, bio); also marks the prior `withdrawn_at` if it was set |
| DELETE | `/v1/community/profile` | Withdraw (sets `withdrawn_at = NOW()`); does NOT delete the row, audit chain stays intact |

All write-side calls are rate-limited (default 5/day per principal)
and append to `trace_contributor_profile_audit`.

## Consent contract

The existing envelope consent scopes (`debugging_evaluation` etc.)
do NOT cover public display. A new scope is required:

- `public_attribution`: contributor authorises their pseudonym to
  be joined to a public handle for display purposes.

The opt-in flow:

1. Contributor calls `PUT /v1/community/profile` with the public
   handle + bio. The request fails if the contributor's
   most-recent upload-claim token doesn't carry the
   `public_attribution` scope.
2. To carry that scope, the issuer must have minted the claim with
   the `--allowed-use public_attribution` flag (operator decides
   when to permit this per-contributor).
3. The contributor MUST acknowledge an in-band consent string at
   opt-in time, captured as a SHA-256 hash in
   `trace_contributor_profile_audit.reason`.

Envelope-level consent doesn't change: existing envelopes don't
suddenly become publicly attributable. The leaderboard counts
existing accepted submissions because the AGGREGATE (count + sum
of credits) is what's published, not the individual envelopes.

## Aggregation guards

Reuse, don't reinvent. The binary already enforces:

- `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` — a leaderboard row is
  suppressed if the contributor's window count is below this
  threshold.
- `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE` — Laplace noise on
  every aggregate count released to the public surface.
- `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_PRIVACY_ACCOUNTING` —
  budget tracking across snapshots.

The snapshot worker calls into these the same way other broad-release
analytics already do. The public read path never sees raw counts.

## Operator gates

- `TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED=false` by default.
  The endpoints return `404` until flipped.
- `TRACE_COMMONS_COMMUNITY_LEADERBOARD_MIN_TENANTS` — refuse to
  publish leaderboard rows when the corpus has fewer than this many
  tenants (pilot stays operator-only until ≥2 tenants).
- `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS`
  — default 900s; tighter means more privacy budget burn.
- Operator can force a takedown for a specific handle by setting
  `withdrawn_at = NOW()` via the existing `/v1/admin/...` reviewer
  surface; the read path picks it up at the next snapshot or via
  a targeted cache flush.

## Implementation slices

Sequenced so each slice ships independently and can be paused at
any boundary.

### Slice 1 — Profile model + opt-in flow

- Migration adding `trace_contributor_profiles` +
  `trace_contributor_profile_audit`.
- `PUT/DELETE /v1/community/profile` endpoints, behind the
  feature flag, requiring `public_attribution` scope.
- Issuer changes to mint claims with the new scope when the
  operator passes `--allowed-use public_attribution`.
- Handle validation + reserved-name list.
- No read-side; no public surface.

This slice is **shippable to the pilot tenant** so operators can
exercise the opt-in flow internally before any data goes public.

### Slice 2 — Snapshot worker + private read surface

- Migration adding `trace_leaderboard_snapshots`.
- Snapshot worker that runs every interval, computes leaderboard
  rows + corpus aggregates, applies min-cell + noise, writes the
  snapshot.
- `GET /v1/community/leaderboard` and `GET
  /v1/community/analytics/summary` endpoints, still behind the
  feature flag, served to the operator surface only.

Pilot operators can verify the snapshots match what the Grafana
dashboard shows (modulo noise) before any public exposure.

### Slice 3 — Public exposure

- Flip `TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED=true` per
  deployment.
- Caddy / load balancer config exposing `/v1/community/...` to
  the public.
- Cache headers (`Cache-Control: public, max-age=300`).
- Operator runbook for handle-takedown, abuse reports, privacy
  budget exhaustion.
- Public-facing static site (separate repo) that consumes
  `/v1/community/...` and renders the leaderboard.

This slice **requires legal/privacy sign-off** before merge. The
gating env stays `false` until that sign-off lands.

### Slice 4 — Verification badges (deferred)

Out of scope. Future slice could let contributors prove ownership
of a GitHub / GitLab / fediverse handle via a challenge-response
flow, with a verified badge on their public profile.

## Migration / rollback

- Slice 1 + 2 are additive: new tables, new endpoints behind a
  feature flag. Rollback = flip the flag, leave the tables.
- Slice 3 is operationally significant: once public, contributor
  handles are visible to the world. Withdrawing the public
  surface means flipping the flag back and invalidating any CDN
  cache. The DB rows persist so a re-enable preserves history.
- Profile rows are not deletable — withdrawal is `withdrawn_at`.
  Hard-delete would break the audit chain.

## Open questions for review

- **Cross-tenant leaderboard.** Should the public leaderboard span
  all tenants by default, or be per-deployment? Current draft is
  per-deployment (the pilot's leaderboard is for the pilot's
  tenants). Cross-deployment federation is a separate, much
  later, design.
- **Handle uniqueness scope.** Per-tenant in the schema as drafted;
  per-deployment-global feels right for display but requires a
  tenant-prefix on the URL (`/v1/community/contributors/<tenant>/<handle>`)
  to avoid impersonation across tenants. Decision deferred to
  reviewers.
- **Metric weighting.** Initial draft is `novelty_credit` as the
  primary metric. Worth confirming that's the right incentive
  (vs. `accepted_count`, vs. a composite) — getting this wrong
  shapes contributor behaviour in the wrong direction.
- **Spam mitigation cost.** The handle validation + reserved-name
  list is the cheap floor. If the pilot sees handle-squatting we
  may need stronger anti-abuse (CAPTCHAs at opt-in, manual review
  for first opt-in per principal). Out of scope until evidence.
