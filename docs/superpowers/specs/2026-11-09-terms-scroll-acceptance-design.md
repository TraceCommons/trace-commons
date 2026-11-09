# Terms of Service: read-through acceptance, re-acceptance, and a server record

Status: draft for owner review, 2026-11-09.

## Intent

A new contributor on the native macOS app should not be able to accept the
Terms of Service without having had the whole text in front of them. The
app loads the current terms live from the Trace Commons site, shows them
inside the Welcome step, and unlocks the accept checkbox only once the
reader has scrolled to the end. The server then keeps a record of which
version of the terms that person accepted, and when, so the acceptance is
attributable later. Loading the text live means the terms can change
without an app release.

When a new version is published after someone has onboarded, the app
stops uploading their traces until they read and accept it in the app.
Everything else keeps working. The stop is the app's own: the server
never refuses an upload over terms.

## The current documents are not valid

The Terms of Service and Privacy Policy published today at
`tracecommons.ai/about/tnc/` and `/about/privacy/` are **not currently
valid** (owner, 2026-11-09). They are drafts, not agreements anyone has
entered. This design builds the mechanism only:

- Do not publish the current text as a valid `terms.json` version, and do
  not treat any record made against it as an acceptance of binding terms.
- Until valid terms are published, the site and the app must say plainly
  that the documents are not yet in effect. Whatever wording does that,
  and where it appears, needs owner approval (copy guide rule 9).
- The first valid version is a separate decision. Shipping this feature
  does not make the current text binding.

## What exists today

- `macos/Sources/TraceCommonsApp/Views/FirstRun/WelcomeScreen.swift`
  (`FirstRunLegalPlaceholder`): a checkbox "I accept the Terms of Service"
  with two links that open `https://tracecommons.ai/about/tnc/` and
  `/about/privacy/` in the browser. The copy struct's comment
  (`first_run_copy.rs:202`) calls it a placeholder, "not a legal
  acceptance receipt".
- `FirstRunRunner.canLeaveWelcome` blocks leaving Welcome until
  `termsAccepted` is true. Nothing is stored or sent.
- The copy (`accept`, `terms`, `privacy`, both URLs) lives in
  `crates/trace-commons-contributor/src/first_run_copy.rs`
  (`WelcomeLegalCopy`), with URLs from `channel::TERMS_URL` /
  `PRIVACY_URL` (production `tracecommons.ai`, staging host for staging).
- The terms page is hand-authored HTML in
  `TraceCommons/trace-commons-community`, `src/pages/about/tnc.astro`
  (Astro 5, Cloudflare Pages, 441 lines). Its only version marker is the
  prose line "Effective 22 August 2026". No machine-readable form exists.
- Acceptance happens on Welcome, before the user has any server identity.
  The first server identity appears at Join (invite enrollment, near.ai
  enrollment, or device-code redeem). A user can also finish first run
  watch-only (`markWatchOnlyComplete`) without signing in at all.
- The daemon is not running on Welcome. First run starts it when roots
  are declared (leaving Tools, or Customize:
  `macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift`, `.startDaemon`),
  and first-run state is not saved to disk.
- Nothing serves the terms on the staging host today:
  `https://staging.tracecommons.ai/about/tnc/` answers 404 (Caddy), and
  the community site deploys production only.
- The terms contain more than prose: a five-row consent-scope table, 12
  inline `<code>` spans, a contents list of in-page anchors, a callout
  box, relative site links and one `mailto:` link.

## Goals

1. The terms text is fetched from the Trace Commons site at first run and
   rendered natively inside the Welcome step.
2. The accept checkbox is disabled until the reader has scrolled the terms
   to the end. Scrolling back up does not re-lock it.
3. On acceptance the client keeps a local record: terms version id,
   SHA-256 of the exact document bytes it rendered, and the acceptance
   time.
4. After the user's first successful sign-in or enrollment, the daemon
   sends that record to the server, which stores it against the
   authenticated account in that org. It does the same for each further
   org the account is used in, and retries until the server confirms.
5. The site publishes the terms once, from one source, as both the
   existing HTML page and the JSON document the app reads, so the two
   cannot drift.
6. When the published version changes after onboarding, uploads stop
   until the person accepts the new version through a notice on the Home
   tab and a row in the menu bar dropdown (owner, 2026-11-09). Accepting
   records it on the server for each org and resumes uploads at once.

## Non-goals

- Server enforcement. The server never refuses an upload because an
  account has not accepted the current terms (owner, 2026-11-09). Older
  app versions keep uploading as before; the acceptance records answer
  who accepted what.
- The Privacy Policy. It stays a plain link, with no scroll requirement
  and no record (owner, 2026-11-09).
- Windows, Linux GTK and Tauri. Native macOS is the lead client; the
  other shells match it later and can reuse the same JSON and daemon
  call.
- Server-side validation that a submitted hash matches a published
  version. The site repo's git history is the authority for what text a
  version id and hash correspond to.
- Fingerprinting the privacy notes and data policy. The terms say both
  "are part of these terms", but the record covers the terms document
  only (owner, 2026-11-09). This is a known limit of the record.
- A staging deployment of the site. Staging builds read the production
  terms (owner, 2026-11-09).

## Anti-goals

- A bundled fallback copy of the terms. It would go stale and the server
  would record a version the user never saw. If the terms cannot be
  loaded, the user cannot continue (fail closed, with a retry).
- An embedded web view.
- Anything in the record beyond version id, hash, times and the
  auth-derived account, tenant, auth method and client kind. No raw URL, no IP, no user agent in
  the stored row or logs (hash-only logging convention).
- Any notice beyond the two the owner asked for (the Home notice and the
  menu bar row in section 5): no system notification, no menu bar icon
  state, no banner elsewhere.

## Approaches considered

- **A. Native view over a JSON document published by the site (chosen,
  owner 2026-11-09).** The site emits `/legal/terms.json` from the same
  source as the HTML page; the app renders it natively; the daemon sends
  version + hash to one new authenticated endpoint. Costs a site change
  and single-sourcing the content. Native look and accessibility, simple
  scroll detection, exact-bytes hash, reusable by every shell.
- **B. Embedded web view of the existing HTML page,** with a script
  reporting the scroll position. No site change, but a browser inside
  first run, mismatched styling, a weak claim about what text was shown,
  and a web view per shell.
- **C. Fetch the HTML and convert it to native text.** Breaks whenever
  the site's markup changes. Dropped.

For the server, one new endpoint called after sign-in was chosen over
adding a field to each of the three enrollment requests: one path to
secure and test, and it also covers the watch-only user who signs in
later.

## Design

### 1. Site: one source, two outputs (trace-commons-community)

- Move the terms body out of `tnc.astro` into a single source file holding
  the version metadata and the content as structured blocks. The page
  renders from it (same URL, same look); a static endpoint renders
  `src/pages/legal/terms.json` from it at build time.
- JSON shape (`schema_version: "trace_commons.terms.v1"`):
  - `version`: an opaque, stable id chosen by whoever edits the terms
    (proposal: the effective date, `"2026-08-22"`, plus a suffix if two
    versions share a day). Changing any content requires changing it; a
    build check fails if the content hash changed but the version did not
    (compare against the previous build's committed `terms.lock`).
  - `effective`: ISO date.
  - `in_effect`: boolean. The current draft text is published with
    `false` (planning call, 2026-11-09, from "the current documents are
    not valid"). A version with `in_effect: false` never starts an upload
    hold; first run still requires scrolling and accepting and shows a
    not-in-effect line. Publishing valid terms means a new version with
    `true`.
  - `title`.
  - `blocks`: ordered list of `{ "kind": "heading" | "paragraph" |
    "list" | "table" | "callout", ... }` (owner chose to extend the
    format rather than edit the terms, 2026-11-09). A `table` has a header
    row and body rows of inline text; a `callout` is a paragraph drawn as
    the page's banner. Inline text is Markdown limited to emphasis, code
    spans and links. The build turns relative site links into absolute
    `https://tracecommons.ai/...` links; `https://` and `mailto:` are the
    only schemes allowed. The page's contents list of in-page anchors is
    not emitted; the app shows headings only.
  - The published terms text does not change: every word on the page
    appears in the JSON. The build check above also verifies this by
    comparing the page's rendered text with the JSON's text.
  - No `hash` field: the client hashes the bytes it received.
- Served as a static file with the site's normal caching. There is no
  staging copy: both channels read the production file.

### 2. Client fetch and render (contributor crate + FFI + macOS)

- New channel constant `TERMS_DOCUMENT_URL`
  (`https://tracecommons.ai/legal/terms.json` in both channels: staging
  has no site, owner 2026-11-09), beside `TERMS_URL`. The fetch's host
  rule is `tracecommons.ai` for both channels.
- The fetch lives in the contributor crate (Rust) so every shell gets the
  same bytes and hash, exposed as a new C ABI call that works before the
  daemon starts (it needs only the channel constants, no state
  directory). It returns either the parsed document plus its SHA-256
  (hex) or a refusal label. The fetch rules mirror the Swift
  `PrivateAICampaignClient` and have to be rebuilt in Rust (there is no
  Rust client to reuse): HTTPS only, the channel's host only, no
  redirects off that host, a size cap (proposal: 256 KiB), a short
  timeout, ephemeral session (no cookies, no cache on disk). An unknown
  `schema_version` or an unparseable document is a refusal.
- `WelcomeScreen`: the legal area becomes a fixed-height scroll region
  rendering the blocks (headings, paragraphs, lists, inline links that
  open in the browser). The checkbox is disabled until the bottom of the
  content has been visible once. The Privacy Policy link stays below it.
  States: loading, loaded, failed (with Try again). Continue stays blocked
  in loading and failed.
- Scroll detection must work at every Dynamic Type size and when the
  whole document fits without scrolling (then it unlocks immediately).
  VoiceOver users reach the end by moving through the content; reaching
  the last element counts as the end.
- `FirstRunRunner.canLeaveWelcome` additionally requires a loaded
  document. `termsAccepted` becomes part of a small accepted-terms value
  (version, hash, accepted time) that the runner holds in memory from
  Welcome and hands to the daemon right after the first successful
  `.startDaemon` (builder's call, 2026-11-09). Quitting before then loses
  it; the next launch restarts first run at Welcome, which asks again.

### 3. Local record (daemon)

- When the runner hands over the accepted terms, the daemon writes
  `terms-acceptance.json` to the
  state directory (0600, atomic write): `version`, `content_sha256`,
  `accepted_at` (RFC 3339, UTC), and `recorded_in`: a list of the tenant
  ids the server has confirmed, starting empty.
- Acceptance is recorded in every org the account is used in (owner,
  2026-11-09). Whenever the daemon holds a session for a tenant that is
  not in `recorded_in` (after a sign-in or enrollment, after an org
  switch, and on daemon start), it posts the record. On a 2xx it adds
  that tenant id and the server's receipt time to `recorded_in`. On a
  transient failure it retries with backoff on the existing poll loop; on
  a 4xx it leaves that tenant unrecorded and logs the label only.
- A second first run in the same state directory replaces the record
  (new version, hash and time, `recorded_in` reset to empty), so the new
  acceptance is posted again to each org.

### 4. Server: endpoint and table (ingest, AGPL)

- Protocol type in `trace-commons-protocol`:
  `TermsAcceptanceRequest { terms_version, content_sha256, accepted_at }`
  (`deny_unknown_fields`), validated: version 1-64 chars of
  `[A-Za-z0-9._-]`, hash 64 lowercase hex, `accepted_at` not more than
  five minutes in the future.
- `POST /v1/account/terms-acceptance` behind the existing account
  middleware (copy the shape of `upload_allowance::handler`). Tenant and
  account come from `AccountCtx` only. `AccountCtx` carries no device
  key, so the row records `auth_method` and `client_kind` instead of a
  device (builder's call, 2026-11-09); deriving a device would be a new
  derivation needing its own security review.
- Migration V158 (next free number at time of writing; re-check before
  merge): `account_terms_acceptances` with `tenant_id`, `account_id`,
  `auth_method`, `client_kind`, `terms_version`, `content_sha256`, `accepted_at`
  (client-claimed), `recorded_at` (server `now()`). Unique on
  `(tenant_id, account_id, terms_version, content_sha256)`; a repeat post
  is idempotent and returns the original `recorded_at`. RLS forced,
  tenant predicate through `trace_current_tenant_id()`, grants matching
  the other account tables. Append-only: no update or delete grant to the
  runtime role.
- Response: `{ "recorded_at": ... }`. Audit/log lines carry labels and
  the version id only.

### 5. New terms after onboarding

**Detection (option 1, owner 2026-11-09).** The daemon re-fetches
`terms.json` on its existing poll loop (proposal: every 6 hours, plus
once at daemon start), with the same fetch rules as first run. If the
published `version` differs from the version in the local record, new
terms are owed. The site is the only source of which version is current;
the server holds no "current version" config. Option 2, the server
announcing the current version in an account-status reply, was rejected:
the version would live in two places that can drift.

**The hold.**
- While new terms are owed, the daemon uploads nothing. Sessions keep
  being observed and queued; queue expiry (`queue_ttl_days`) and the
  entry cap's eviction do not run against entries waiting only because of
  the hold, so nothing is lost to it.
- The hold is its own state, separate from the user's pause: the
  existing Resume does not clear it, and accepting does not resume a
  pause the user set themselves.
- A failed fetch never starts a hold. Uploads continue under the last
  accepted version; a site outage must not stop contribution.
- Watch-only users (never signed in) upload nothing anyway. They still
  get the notice, so they accept before they first sign in.
- Users who onboarded before this feature have no local record. Their
  first successful fetch of a valid version owes it, so the first valid
  terms published move every existing user onto them through this flow.

**Accepting.** One daemon call takes the version and hash the app
rendered (fetched again when the sheet opens, so the person accepts what
they read). It replaces the local record (new version, hash, time,
`recorded_in` empty), lifts the hold at once, and the records are posted
to each org as in section 3. If the version the app read is no longer
the published one, the call refuses and the sheet reloads.

**Home tab notice** (owner, 2026-11-09). A notice in Home's popover
stack (`MonitorNotices`, place `.home`) with no close button, like the
consent gate hold: it goes only when its cause does. It names the new
terms, says uploads are paused until the person accepts, and has one
button that opens a sheet. The sheet reuses the first-run terms view:
loading and failed states, scroll to the end to unlock the checkbox,
then Accept, with the Privacy Policy link below. Closing the sheet
without accepting leaves the notice. There is no Not now.

**Menu bar dropdown** (owner, 2026-11-09). The same state as a row in
`MenuBarGlassPanel`'s nudge slot. Clicking it opens the window on Home
with the sheet open. The dropdown's bar graph greys as it does while
paused.

Nothing else is added: no system notification, no icon state, no other
tab. The Traces tab's existing paused state covers the rest.

### 6. Copy

Strings below are DRAFT, NEEDS APPROVAL, in `WelcomeLegalCopy`, following
`docs/ui-copy-guide.md`:

- checkbox (unchanged): "I accept the Terms of Service"
- disabled-checkbox hint: "Scroll to the end to accept"
- loading: "Loading the Terms of Service"
- failed: "Couldn't load the Terms of Service. Check your connection."
- retry button: "Try again"
- Home notice: "New Terms of Service. Uploads are paused until you
  accept."
- Home notice button: "Review terms"
- menu bar row: "Uploads paused: new Terms of Service"
- sheet's Accept button: "Accept"

The Home notice and menu bar row are new notices: the owner asked for
them (2026-11-09), and their final wording still needs owner approval
(copy guide rule 9).

## Testing

- Site: build test that `terms.json` matches the page source and the
  version-bump check fails on changed content with an unchanged version.
- Contributor: unit tests for the fetch refusals (wrong host, redirect,
  oversize, bad schema), the hash over exact bytes, and the local record
  rules (each org confirmed once; 4xx leaves that org unrecorded; replace on a second first run).
- Server: endpoint tests for auth required, validation, idempotent
  repeat, tenant isolation under RLS (PostgreSQL suite), and the
  license boundary unchanged.
- macOS: runner tests that Continue is blocked while loading, failed, and
  unscrolled; unlocked after the end is reached; unlocked at once for a
  document that fits. FFI build before `swift test`.
- Hold: a changed published version starts the hold and nothing
  uploads; a failed fetch does not start one; queue expiry and cap
  eviction skip held entries; Resume does not lift it; accepting lifts
  it, does not lift a user pause, and refuses a version that is no longer
  published; a user with no record is held by the first valid version.
- macOS: the Home notice and menu bar row appear only while held, have
  no close action, and open the sheet; accepting in the sheet removes
  both.

## Left to the builder

- Exact Astro mechanism for the single source (content collection, a
  `.ts` module, or frontmatter), as long as page and JSON come from one
  file.
- SwiftUI scroll-end detection technique, within the macOS 14 floor.
- Backoff schedule, and the exact FFI function name and JSON shape, in
  line with neighbouring calls.
- Where in the watcher and queue the hold and the expiry exemption hook
  in, and the re-check interval (6 hours proposed).
