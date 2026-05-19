# Trace Commons Community Frontend — Design

Date: 2026-05-19
Status: Draft (pre-implementation)

Owner: Trace Commons / Community surface

## Decision frame

The leaderboard API design
([`./2026-05-19-community-analytics-leaderboard-design.md`](./2026-05-19-community-analytics-leaderboard-design.md))
defines the public, opt-in `/v1/community/...` surface on the server.
This spec defines its **first consumer**: a community-facing website
that surfaces the leaderboard, per-contributor profiles, and
aggregate corpus analytics in a form the community can read, browse,
and link to.

The decision frame separates two distinct surfaces, sharing one
backend:

- **Read surface (default)**: everything public — leaderboard,
  contributor pages, corpus dashboards. Static, CDN-friendly,
  generated from snapshot JSON.
- **Write surface (small, authenticated)**: contributor profile
  management (opt in, set handle, edit bio, withdraw). Tiny SPA
  island authenticated against the existing upload-claim issuer
  flow. Used by maybe a few hundred contributors over the life of
  the pilot.

Co-locating both in one stack and one repo is fine; conflating their
trust posture is not. The read surface treats data as already
publishable; the write surface is the only place that handles a
contributor's authentication material.

## Goal

A separate frontend repo (working name `trace-commons-community`)
that:

1. Renders the public leaderboard and per-contributor pages at a
   cacheable URL, fast enough to be the obvious place to link to
   when bragging about a contribution.
2. Renders the corpus aggregate dashboards (volume, accept rate,
   novelty distribution, gate-decision breakdown) in a form a
   non-operator can read without context, subject to the same
   min-cell / noise guards already enforced server-side.
3. Provides an opt-in / handle / bio management UI for contributors
   to manage their own public profile, authenticated against the
   issuer.
4. Is deployable independently of the server — frontend changes
   ship without touching the Rust binary, and vice versa.
5. Has zero access to operator-only surfaces. The operator
   dashboard ([`../../operator/pilot-dashboard.md`](../../operator/pilot-dashboard.md))
   stays in Grafana behind operator auth; this site never talks to
   Cloud SQL.

## Non-goals

- Operator dashboards, admin tooling, reviewer UIs. Those are
  separate surfaces with separate trust postures; the community
  site never has the right credentials to touch them.
- Per-trace inspection. The site shows aggregates and per-contributor
  totals. Showing envelope content (even of opted-in contributors)
  is a separate spec.
- A research-access / corpus-download portal. Bulk corpus access for
  research is a much larger surface (consent verification, license
  agreement, throttling) and lives in its own future spec.
- Comments, follows, messaging, badges. The first cut is
  one-direction: server publishes, site renders. Social features
  belong in a later iteration after evidence the surface is being
  used.
- A mobile app. Browser-only. The leaderboard is read-mostly and
  designed for any modern browser.
- Real-time anything. Snapshots refresh on the server's interval
  (default 15 min per the leaderboard spec); the site never
  polls more frequently than the snapshot interval.

## Current shape

Nothing exists. The leaderboard API design is itself unmerged (PR
#114). This spec is forward-looking: it describes the consumer the
API should expect, so the API spec can be reviewed knowing how it
will be used.

## Architecture

```
GitHub Pages / CDN
        |
        v
  trace-commons-community (Astro)
        |
        +-- static pages (build-time + 15-min revalidate)
        |       GET https://ingest.<pilot>/v1/community/leaderboard
        |       GET https://ingest.<pilot>/v1/community/analytics/summary
        |       GET https://ingest.<pilot>/v1/community/contributors/{handle}
        |
        +-- /profile SPA island (client-side only, behind login)
                Auth: same workload-token -> upload-claim flow used by ironclaw
                PUT  https://ingest.<pilot>/v1/community/profile
                DELETE https://ingest.<pilot>/v1/community/profile
```

Read paths are pre-rendered HTML built from snapshot JSON at build
time (and incrementally revalidated every 15 min). Write paths are
a small SPA mounted at `/profile` that hits the server directly
from the browser using an authenticated session.

### Why Astro

The right default unless reviewers argue for an alternative:

- Static-first with optional islands — matches the read-mostly /
  small-write-island shape.
- Vanilla TypeScript components for the islands; no framework
  monoculture lock-in.
- Easy CDN deploy (GitHub Pages, Cloudflare Pages, Vercel,
  Netlify all work without server runtime).
- Markdown content for the static intro pages, sharing the same
  Markdown tooling the rest of the docs use.

Alternatives considered:

- **Next.js**: more frontend mass to maintain, ISR is fine but
  requires a Node runtime to host. Avoid unless reviewers prefer
  React-everywhere.
- **Hugo / Jekyll**: zero islands story — the profile-management
  surface needs JS regardless.
- **Pure SPA (Svelte/React)**: more uniform but throws away the
  CDN cacheability of the read surface, and the read surface is
  the part that matters most.

## Route map

```
/                          Landing: what is Trace Commons, where the
                           data comes from, link to ironclaw setup.
/leaderboard               Default view: top contributors by
                           novelty_credit (7d window). Window /
                           metric selectors are static links to
                           pre-rendered variants.
/leaderboard/30d           Pre-rendered 30d snapshot.
/leaderboard/all           Pre-rendered all-time snapshot.
/contributors/{handle}     Per-contributor profile: handle, bio,
                           public_since, totals, rolling-window
                           stats. 404 if not currently public.
/analytics                 Aggregate corpus dashboards: volume over
                           time, accept rate, novelty histogram,
                           gate-decision distribution.
/profile                   SPA island. Requires auth. UI for opt
                           in / edit handle / edit bio / withdraw.
/profile/auth-callback     Handles the upload-claim issuer's
                           consent-return flow.
/about/privacy             What's published, what isn't, how
                           opt-in works, how withdrawal works.
/about/data-policy         Consent scopes, min-cell guards, noise
                           policy — link to the relevant operator
                           docs.
```

All pages render with a stable HTML structure so search engines and
social-preview unfurlers can summarise them. JSON-LD on the
contributor pages so a handle's stats appear nicely in chat
previews.

## Auth flow (write surface only)

The leaderboard write side reuses the existing two-step issuer
flow with one addition — the `public_attribution` consent scope.

```
1. Contributor visits /profile.
2. SPA detects no session, redirects to upload-claim issuer with
   redirect_uri=https://community.<pilot>/profile/auth-callback,
   scope=public_attribution.
3. Issuer prompts the contributor to consent to public attribution
   (this is the load-bearing consent step), then redirects back
   with a short-lived upload-claim token.
4. SPA stores the token in sessionStorage (NOT localStorage —
   minimise persistence), uses it as the Bearer on
   PUT/DELETE /v1/community/profile.
5. Token expires (default 5 min per upload-claim TTL); SPA
   surfaces "session expired, click here to re-auth" rather than
   silently refreshing.
```

No long-lived cookies, no OAuth refresh tokens, no first-party
session backend. The token's TTL IS the session. This is uglier UX
than a typical web app and intentional: the consent moment is
re-stated every session, the surface that can mint profile updates
is short-lived, and there's no server-side session state to leak.

The site uses `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` so the auth window
can't be hijacked.

## Build pipeline

```
+-- pre-build: fetch snapshots from /v1/community/...
|       Cache JSON under src/_data/.
|       Hash + commit-pin the snapshot id so the build is
|       reproducible from the snapshot id.
|
+-- astro build: render static HTML against the cached JSON.
|       Per-contributor pages generated for every opted-in handle
|       in the snapshot.
|
+-- post-build: emit a build manifest (snapshot id, build sha,
|       built_at) for the deploy step to verify.
|
+-- deploy: push to CDN. CDN serves with
|       Cache-Control: public, max-age=900, stale-while-revalidate=300.
|       Trigger: GitHub Actions cron every 15 min OR webhook from
|       the snapshot worker (Slice 3 of the leaderboard spec).
```

The build is a pure function of the snapshot JSON and the source
code. No DB access from CI. No secrets in the build.

CI must:
- Validate the JSON schema of every fetched snapshot (so a server
  regression doesn't render a broken site).
- Fail closed if the public-flag (`TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED`)
  is `false` on the upstream — the site refuses to deploy an empty
  surface.
- Reject snapshots older than the configured staleness threshold.

## Slices

### Slice 1 — Repo + landing + dummy leaderboard

- New repo `trace-commons-community`. Astro skeleton, CI, deploy
  pipeline, landing page, docs/privacy pages.
- Leaderboard route renders against committed dummy snapshot JSON
  (no server dependency).
- Per-contributor route renders against committed dummy data.
- Operator can preview the layout and copy before any real
  contributor sees their handle in production.

Ships independently of the server. The Rust binary doesn't change.

### Slice 2 — Live read against the API

- Build pipeline fetches real snapshots from a pilot
  `/v1/community/...` endpoint (gated by the operator flag).
- Pre-renders per-contributor pages for every opted-in handle in
  the snapshot.
- Deploys to a public URL behind a maintenance banner.

Requires the leaderboard spec Slice 2 to have shipped (snapshot
worker + read endpoints).

### Slice 3 — Profile SPA island

- Adds `/profile` route with the SPA island.
- Implements the issuer redirect flow.
- Implements PUT / DELETE against `/v1/community/profile`.
- Inline preview of the user's would-be public page.

Requires the leaderboard spec Slice 1 to have shipped (profile
write endpoints).

### Slice 4 — Polish + analytics dashboards

- `/analytics` route with the corpus aggregate dashboards
  (volume, accept rate, novelty distribution, gate-decision
  breakdown), rendered from the analytics summary snapshot.
- Social-preview unfurls (Open Graph + Twitter Card tags).
- Per-contributor JSON-LD.
- i18n hook (English only at launch; structure ready for
  community-contributed translations later).

### Slice 5 — Public launch

- Removes the maintenance banner.
- Operator flips the public flag on the server.
- CDN config tightened: HSTS, CSP, Referrer-Policy strict.
- Operator runbook in `trace-commons-server` updated to point at
  the public URL.

Requires the leaderboard spec Slice 3 (public exposure on the
server side) + legal/privacy sign-off.

## Threat model (frontend-specific)

The frontend faces a narrower threat surface than the server (it
ships no secrets, no DB access), but it has its own concerns:

- **XSS via contributor-controlled bio.** Bio is rendered as
  plaintext, never as HTML. Length capped server-side and
  re-validated client-side. No Markdown rendering on the public
  surface at launch.
- **Auth-window hijack on the write surface.** COOP/COEP headers,
  short-lived tokens in sessionStorage only, no third-party
  scripts on `/profile` routes.
- **CSP bypass via embedded third-party content.** Strict CSP
  excluding inline scripts, no third-party CDN for fonts /
  analytics. Self-hosted everything.
- **Snapshot mid-air rewrite.** CI verifies the snapshot
  signature (TBD with the leaderboard spec — open question whether
  snapshots are signed) before building.
- **Search engine indexing of withdrawn profiles.** When a profile
  is withdrawn server-side, the next build returns 404 +
  `X-Robots-Tag: noindex`. The cache TTL bounds the worst-case
  exposure to the snapshot interval.
- **Operator UI confusion.** The site does NOT carry any link to
  operator/admin surfaces. Operator tooling lives at a different
  hostname behind operator auth.

## Open questions for review

- **Hosting.** GitHub Pages, Cloudflare Pages, Vercel, Netlify all
  fit. GH Pages is simplest if the repo is in the same GH org;
  Cloudflare Pages gives better cache controls and edge functions
  if we need them later. No strong default — operator preference.
- **Domain.** `community.trace-commons.org` is the natural shape;
  the actual domain depends on org/branding decisions outside
  this spec.
- **Snapshot signing.** Open in the leaderboard spec — if
  snapshots are signed, the frontend verifies the signature
  before building. If not, CI relies on transport-layer trust to
  the issuer.
- **Profile preview shareability.** Should the per-contributor
  page have a "share to X / Bluesky" button? Useful for
  community engagement; adds tracking-pixel risk if implemented
  carelessly. Default no, revisit after launch.
- **Analytics-on-analytics.** Should the site itself collect
  page-view analytics? Default no (privacy posture + zero
  dependency on third-party analytics). Server access logs at
  the CDN edge are sufficient for capacity planning.
- **Single-pilot vs multi-deployment.** Initial cut is one
  frontend per deployment (a community site per Trace Commons
  pilot). A federated "see all Trace Commons deployments"
  surface is a separate, much later, design.

## Out of scope (named so they don't get scope-crept in)

- A research portal for downloading the corpus.
- Reviewer/moderator dashboards.
- Admin tooling.
- Per-tenant micro-sites with custom branding.
- Identity verification (GitHub OAuth, etc.) — deferred to a
  later slice on the server side.
- Notification subscriptions ("email me when my rank changes").
- Comments, follows, messaging.
- Mobile apps.
