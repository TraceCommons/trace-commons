# `tracecommons.ai` community site runbook

This runbook covers the public pilot surface we own on Cloudflare Pages:
the pseudonymous leaderboard, contributor profiles, and aggregate corpus
analytics for invited Ironclaw contributors.

The static assets live in [`../../community/`](../../community/). The live
browser data path is same-origin `/api/v1/community/*`, served by the
Cloudflare Pages Function in
[`../../community/public/_worker.js`](../../community/public/_worker.js) and
proxied to `https://ingest.tracecommons.ai/v1/community/*`.

## Admin flow

1. Deploy the ingest and issuer hosts with community onboarding URLs:

   ```sh
   TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED=true
   TRACE_COMMONS_COMMUNITY_CORS_ORIGINS=https://tracecommons.ai
   TRACE_COMMONS_ONBOARDING_COMMUNITY_URL=https://tracecommons.ai
   TRACE_COMMONS_ONBOARDING_PROFILE_URL=https://tracecommons.ai/profile
   TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL=https://tracecommons.ai/leaderboard
   ```

   Keep the local preview origins from
   [`../../deploy/pilot-gcp/ingest.env.template`](../../deploy/pilot-gcp/ingest.env.template)
   in staging if you need direct browser testing from `127.0.0.1:8788`.

2. Create the Cloudflare Pages project from `community/`:

   ```sh
   cd community
   npm run check
   ```

   Use build command `npm run render:aasa && npm run check` (with
   `TC_APPLE_TEAM_ID` set in the Pages build environment) and output directory
   `public`. Attach custom domain `tracecommons.ai`.
   The repo also carries `community/wrangler.toml` for direct uploads:

   ```sh
   cd community
   TC_APPLE_TEAM_ID=<TEAMID> npm run deploy:pages
   ```

   This command requires Cloudflare credentials in the operator environment.
   It renders the AASA file first (see below) and refuses to deploy if
   `TC_APPLE_TEAM_ID` is unset or malformed.

3. Edit `community/public/experience.json` for the current cohort prompt,
   milestone targets, and weekly rhythm. This is the participant-facing
   brief at `https://tracecommons.ai/brief`.

4. Seed invite codes with the batch helper in
   [`./pilot-allowlist.md`](./pilot-allowlist.md). For the initial cohort,
   one invite per contributor keeps troubleshooting simple; use the default
   retry budget so client retries do not burn the whole invitation.

5. Hand-provision each candidate over a private Slack DM or equivalent.
   Send only the invite link, the expected `ironclaw traces onboard`
   command, and the privacy reminder. Do not post raw invite codes in a
   shared channel.

6. Smoke one invite end-to-end:

   ```sh
   ironclaw traces onboard '<invite-link>'
   ironclaw traces preview --recorded-trace tests/fixtures/llm_traces/recorded/weather_sf.json --enqueue
   ironclaw traces flush-queue
   ```

7. Recompute the community snapshot after accepted traces land:

   ```sh
   curl -sfS -X POST \
     -H "authorization: Bearer $TRACE_COMMONS_ADMIN_TOKEN" \
     https://ingest.tracecommons.ai/v1/admin/community/snapshots/recompute
   ```

8. Check the public surface:

   ```sh
   curl -sfS https://ingest.tracecommons.ai/v1/community/leaderboard
   curl -sfS https://tracecommons.ai/api/v1/community/leaderboard
   curl -sfS https://tracecommons.ai/leaderboard
   curl -sfS https://tracecommons.ai/analytics
   curl -sfS https://tracecommons.ai/brief
   ```

## Contributor flow

The contributor-facing version is
[`./pilot-contributor-onboarding.md`](./pilot-contributor-onboarding.md).
The short form is:

1. Receive private invite link.
2. Run `ironclaw traces onboard '<invite-link>'`.
3. Submit a metadata-only fixture trace.
4. Check `ironclaw traces credit` and queue status.
5. Ask Ironclaw to set a pseudonymous public profile handle, or copy a
   short-lived public-attribution token from Ironclaw into the browser
   profile page.
6. Open `https://tracecommons.ai/profile` to review or withdraw the public
   handle, and watch `https://tracecommons.ai/leaderboard` after the next
   snapshot.
7. Open `https://tracecommons.ai/brief` for the current trace prompt and
   cohort milestones.

Current invite onboarding grants the device key both normal pilot trace
capability and the separate `public_attribution` profile-management
capability by default. The browser page never asks for the device private key
or workload JWT. If a participant is on an older fallback build, keep the
workload JWT in their shell environment and rotate it manually.

## Rich pilot loop

The experience should feel alive after onboarding, not like a one-time
submit form.

- Run a daily snapshot refresh during the first week so contributors see
  movement quickly.
- Post a short cohort prompt in Slack and mirror it in
  `community/public/experience.json`: one suggested workflow to trace, the
  current top handle, and the aggregate acceptance rate.
- Keep the leaderboard rolling-window based. This gives late joiners room to
  appear without permanently chasing the first-day uploaders.
- Encourage pseudonymous bios that describe agent habits or tool specialties,
  not legal identity.
- Use aggregate analytics for shared progress: acceptance rate, novelty
  distribution, and gate outcomes. Do not discuss raw trace contents in the
  public channel.
- Review quarantine at least twice per week while the cohort is small. Tell
  contributors whether a stalled credit is waiting on privacy review or is a
  duplicate.
- At the end of each week, share a small recap: public handle count,
  accepted traces, top novelty movement, and one next prompt.

## DM packet

Use this shape for manual provisioning:

```text
You are invited to the TraceCommons internal pilot.

1. Update Ironclaw to current main.
2. Run: ironclaw traces onboard '<invite-link>'
3. Submit one fixture trace, then set a pseudonymous handle at:
   https://tracecommons.ai/profile

Please do not use your legal name, email, Slack handle, or account id as the
public handle. Leave message text and tool payload sharing off for the first
submission so it can auto-accept.
```

## Launch checks

- `cd community && npm run check` passes.
- `https://tracecommons.ai/brief` renders the current `experience.json`
  prompt and cohort milestones.
- `TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED=true` is live on ingest.
- `https://tracecommons.ai/api/v1/community/leaderboard` returns live ingest
  JSON with `x-tracecommons-proxy: community`.
- Issuer onboarding response includes `profile_url` and `leaderboard_url`.
- First accepted submission appears after snapshot recompute.
- Withdraw profile flow removes the contributor after the next snapshot.
- `scripts/check-aasa.sh https://tracecommons.ai` passes (see below).

## Apple app-site-association (native passkeys)

The macOS app creates and uses passkeys for the relying party
`tracecommons.ai`. macOS only allows that after fetching
`https://tracecommons.ai/.well-known/apple-app-site-association` (through
Apple's CDN) and finding the app's ID in it. Design:
`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`
(Z2 of #1118, slice S4).

**How it is produced.** `community/scripts/render-aasa.mjs` writes
`community/public/.well-known/apple-app-site-association` (git-ignored) from:

| Variable | Meaning |
| --- | --- |
| `TC_APPLE_TEAM_ID` | Required. The Team ID of the Developer ID certificate that signs the app: exactly 10 uppercase alphanumerics. Not a secret. |
| `TC_MACOS_BUNDLE_ID` | Optional, default `ai.tracecommons.shell` (`macos/scripts/info-plist.sh`). |

The body is exactly `{"webcredentials":{"apps":["<TEAMID>.ai.tracecommons.shell"]}}`.
There is no `applinks` section. With `TC_APPLE_TEAM_ID` unset or malformed the
render step deletes any stale file, writes nothing and exits non-zero, and
`npm run deploy:pages` stops there. The site never ships a placeholder.

**How it is served.** `community/public/_worker.js` answers `/.well-known/*`
itself, before its SPA fallback. Without that, a missing AASA (no dot in the
last path segment) would be returned as `index.html` with a `200`. The worker
returns the file with `Content-Type: application/json` and
`Cache-Control: public, max-age=3600`, and returns a real `404` for anything
under `/.well-known/` that is not a plain `200` asset, redirects included.
`_headers` carries the same two headers as a second line of defence.

**Post-deploy smoke.** Run this after every deploy, and after any change to the
worker or `_headers`:

```sh
scripts/check-aasa.sh https://tracecommons.ai
# or pin the exact app id:
scripts/check-aasa.sh https://tracecommons.ai "<TEAMID>.ai.tracecommons.shell"
```

It requires `200`, no `Location` header (redirects are never followed),
`Content-Type: application/json`, valid JSON of exactly the shape above, and an
app id matching `^[A-Z0-9]{10}\.ai\.tracecommons\.shell$` (or equal to the
pinned value). It needs `curl` and `jq`. `cd community && npm test` covers the
renderer (unset and malformed Team IDs are refused), the worker route, and this
script against a local server; the `community site (AASA)` CI job runs them.

**macOS side (native team, not done here).** The app needs the entitlement

```xml
<key>com.apple.developer.associated-domains</key>
<array><string>webcredentials:tracecommons.ai</string></array>
```

and must be signed with a Developer ID certificate of the Team in
`TC_APPLE_TEAM_ID`, with a provisioning profile that carries associated
domains. An ad-hoc or unsigned build cannot use the association, so passkey
calls fail closed there. Whether a Developer ID profile carries `webcredentials`
for a non-App-Store app is still to be confirmed (spec, open questions). A
Tauri build would need its own entitlement and an additional `apps` entry.
