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

   Use build command `npm run check` and output directory `public`. Attach
   custom domain `tracecommons.ai`.
   The repo also carries `community/wrangler.toml` for direct uploads:

   ```sh
   cd community
   npm run deploy:pages
   ```

   This command requires Cloudflare credentials in the operator environment.

3. Edit `community/public/experience.json` for the current cohort prompt,
   milestone targets, and weekly rhythm. This is the participant-facing
   brief at `https://tracecommons.ai/brief`.

4. Seed invite codes with the allowlist flow in
   [`./pilot-allowlist.md`](./pilot-allowlist.md). For the initial cohort,
   one invite per contributor keeps troubleshooting simple.

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
