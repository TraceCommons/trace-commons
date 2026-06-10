# Pilot contributor onboarding

This doc walks a new contributor (typically an Ironclaw dev) through joining
the hosted TraceCommons pilot — from receiving credentials, through
configuring their Ironclaw client, to submitting their first trace.

If you are the operator bringing up the pilot environment instead, see
[`./pilot-gcp-deployment.md`](./pilot-gcp-deployment.md).

## What this gets you

You'll be able to submit redacted agent traces from your local Ironclaw
into the shared pilot corpus. Accepted traces earn novelty credits, and
the redacted envelopes land in the project's CMEK-encrypted GCS bucket.
Raw message text and tool payloads stay on your machine; only the
redacted envelope leaves.

## What you receive from the operator

The operator hands you an invite link over an operator-approved private
channel. For the first internal pilot this may be a manual Slack DM or
similar direct handoff; do not post it in a shared channel.

The link carries an invite code and the issuer URL. The invite code is
16 chars, e.g. `FMJD93VFPROQ5I4I`. The issuer hashes this with sha256
and checks against its allowlist; the raw code never lands in the DB.
Ironclaw uses the invite once to register a local device public key
through `POST /v1/onboard`.

The onboarding response gives Ironclaw the rest of the pilot config:

- Tenant ID, usually `tenant-zaki-pilot` for the first cohort.
- Ingest: `https://ingest.tracecommons.ai`
- Issuer: `https://issuer.tracecommons.ai`
- Community profile: `https://tracecommons.ai/profile`
- Public leaderboard: `https://tracecommons.ai/leaderboard`
- Device key id: `sha256:<64-hex>`, derived from your local public key.

The response is safe for Ironclaw to store in its local contribution
profile and expose to the agent. It contains only tenant/config labels,
public URLs, and hash-derived device identity.

After onboarding, Ironclaw signs each upload-claim request with the
local device key. The issuer verifies the `x-trace-device-key-id` and
`x-trace-device-signature` headers before returning the short-lived
Bearer upload claim used by ingest. Older pilot clients may still use a
temporary workload JWT fallback; treat that JWT as a bearer secret and
keep it in your shell environment only.

## Prerequisites

- Ironclaw built from a recent `main` (PR #3738 or later). Confirm
  with `ironclaw --version`.
- Whatever LLM provider config you already use with Ironclaw — the
  trace pipeline is independent of inference, it just observes what
  your agent did.

## Configure Ironclaw

Trace contribution is a local-first, opt-in feature. There is no TOML
config block to edit; Ironclaw writes a standing policy under
`~/.ironclaw/trace_contributions/`.

Preferred pilot flow:

```sh
ironclaw traces onboard '<invite-link-from-operator>'
```

The Ironclaw onboarding command should:

1. Generate or load a local Ed25519 device key.
2. POST the invite code, base64 public key, and client info to
   `https://issuer.tracecommons.ai/v1/onboard`.
3. Persist the returned tenant id, ingest URL, issuer URL, audience,
   device key id, profile URL, and leaderboard URL.
4. Enable the standing contribution policy for the chosen consent
   scope, usually `debugging-evaluation`.

If your Ironclaw build still predates the registered device-key flow,
use the temporary workload-JWT fallback:

```sh
export IRONCLAW_TRACE_WORKLOAD_TOKEN='<jwt-from-operator>'

ironclaw traces opt-in \
  --endpoint https://ingest.tracecommons.ai \
  --upload-token-issuer-url https://issuer.tracecommons.ai \
  --upload-token-issuer-allowed-hosts issuer.tracecommons.ai \
  --upload-token-audience trace-commons-ingest \
  --upload-token-tenant-id tenant-zaki-pilot \
  --upload-token-workload-token-env IRONCLAW_TRACE_WORKLOAD_TOKEN \
  --upload-token-invite-code '<invite-code-from-operator>' \
  --scope debugging-evaluation
```

Notes:
- `--upload-token-issuer-allowed-hosts` is a hostname allowlist Ironclaw
  enforces on the issuer URL. It protects against misconfigured DNS.
- `--upload-token-invite-code` is required on the fallback path because
  the pilot issuer runs with `TRACE_COMMONS_ALLOWLIST_SOURCE` set.
- `--scope` controls which uses your envelopes consent to.
  `debugging-evaluation` covers debugging, evaluation, and aggregate
  analytics.
- Don't pass `--include-message-text` or `--include-tool-payloads`
  unless you've explicitly decided to share them — both stay off by
  default. The pilot accepts envelopes either way, but envelopes that
  include message text or tool payloads route to the privacy-review
  queue instead of auto-accepting; see [Accepted vs. quarantined
  outcomes](#accepted-vs-quarantined-outcomes).
- Your `pseudonymous_contributor_id` is generated locally and stable
  per scope. You can override per-preview with `--contributor-id`, but
  there's no need.

Confirm the policy landed:

```sh
ironclaw traces status
```

## Submit your first trace

Use a recorded fixture to confirm the chain end-to-end before you
generate trace traffic from real sessions:

```sh
ironclaw traces preview \
  --recorded-trace tests/fixtures/llm_traces/recorded/weather_sf.json \
  --enqueue
ironclaw traces flush-queue
```

A successful submit prints one of two outcomes. With the defaults
(no `--include-message-text`, no `--include-tool-payloads`):

```
{"status":"accepted","credit_points_pending":5.2,"explanation":["Accepted into the private redacted corpus.","Attributed to tenant tenant_sha256:..."]}
```

With message text or tool payloads included:

```
{"status":"quarantined","credit_points_pending":0.0,"explanation":["Quarantined for privacy review; credit is pending review.","Attributed to tenant tenant_sha256:..."]}
```

Both are successes — the trace is stored and attributed in both cases.
See [Accepted vs. quarantined outcomes](#accepted-vs-quarantined-outcomes)
for what happens next in each lane.

`credit_points_pending: 0.0` on the accepted lane means you've already
submitted that exact content (the gate suppresses duplicates).

Check what landed:

```sh
ironclaw traces list-submissions
ironclaw traces credit
ironclaw traces queue-status
```

## Set your public pilot handle

The leaderboard uses your local pseudonymous contributor ID as the stable
subject, then lets you opt in to a display handle. After onboarding,
Ironclaw should expose the profile URL returned by `/v1/onboard`, ask for
your preferred pseudonymous handle, and manage the profile with the local
device key. If you open the profile page directly:

1. Open `https://tracecommons.ai/profile`.
2. Paste the public-attribution token generated by Ironclaw. This is a
   short-lived Bearer upload claim scoped to profile management; do not
   paste your device private key or workload JWT into the browser.
3. Enter your chosen display handle and optional bio.
4. Save the profile. Withdrawing the profile from the same page removes
   the public handle after the next snapshot recompute.

The browser profile page does not sign device-key requests. Ironclaw keeps
the local device key on your machine and uses it to request the
public-attribution token from the issuer. Current invite onboarding grants
that profile-management capability by default. Older fallback builds may
still ask for workload-JWT context; keep that JWT in your shell environment
only.

Do not use a legal name, email address, Slack handle, account id, or
anything else that would defeat the pseudonymous leaderboard. During the
pilot, the operator can manually revoke or rotate a handle if someone
picks one they later regret.

## What appears on the community site

`https://tracecommons.ai/leaderboard` shows the rolling 7-day ranking for
accepted traces. It starts empty and updates only after accepted
submissions exist and the community snapshot is recomputed by the server.
`https://tracecommons.ai/analytics` shows aggregate counts for the same
window. `https://tracecommons.ai/brief` shows the current cohort prompt,
milestone targets, and operator cadence for the week.

If you are in the auto-accept lane, you should expect a loop like this:

1. Submit redacted traces locally with Ironclaw.
2. Flush the queue.
3. Check `ironclaw traces credit` for pending credit.
4. Register or update your display handle on the profile page.
5. Check the brief for the next suggested workflow.
6. Watch the leaderboard after the next snapshot refresh.

## Accepted vs. quarantined outcomes

The pilot routes every successful submission into one of two lanes
based on the envelope's residual PII risk:

| Lane | When | Credit | What happens next |
|---|---|---|---|
| `accepted` | Metadata-only envelope (no message text, no tool payloads) | `credit_points_pending` populated immediately; settles after the gate worker scores it | Trace lands in the private redacted corpus and is available to downstream consumers |
| `quarantined` | Envelope opted into `--include-message-text` or `--include-tool-payloads` | Held at `0.0` until a human reviewer processes it | Operator reviews the envelope and either releases it (credit mints) or revokes it (tombstone) |

Quarantine is a privacy posture, not a punishment. The pilot treats
any envelope that carries body text or tool payloads as needing a
human eye before that content joins the shared corpus — there is
currently no automatic downgrade path even after server-side
re-scrubbing. The policy decision behind this (and whether to wire
an auto-accept path off a successful privacy-filter pass) is tracked
in [#131](https://github.com/TraceCommons/trace-commons-server/issues/131).

Expected wait for the quarantine queue depends on operator capacity;
ask the operator if your `credit_points_pending` has been at `0.0`
for more than a few days. You can keep submitting in the meantime —
each envelope is scored independently.

To keep your traces in the auto-accept lane, leave
`--include-message-text` and `--include-tool-payloads` off. You get
less detailed traces, but they earn credit without manual review.

## Day-to-day usage

Once you're opted in, recorded sessions automatically enqueue redacted
envelopes under `~/.ironclaw/trace_contributions/queue/`. Ironclaw
flushes on its own schedule, or you can force a flush:

```sh
ironclaw traces flush-queue
```

To preview what a redacted envelope looks like before sending:

```sh
ironclaw traces preview --recorded-trace <session.json>
```

(Omit `--enqueue` to just inspect.) The preview is exactly what would
be uploaded.

## What you're agreeing to

Each envelope embeds the consent policy version (`2026-04-24` at the
time of writing) and the allowed-uses list. By submitting you agree
that the redacted envelope can be used for the purposes covered by
your `--scope` (default `debugging-evaluation`: debugging, evaluation,
and aggregate analytics).

The redacted envelope does NOT include (unless you explicitly opt in
via `--include-message-text` / `--include-tool-payloads`):

- Raw user or assistant message text.
- Raw tool call/response payloads.
- API keys, environment variables, or any other secret material.

It DOES include:

- The sequence of tool kinds used (e.g. `http`, `shell`, `file`).
- Hashes of the input/output of each tool call (not the bodies).
- Outcome labels (`success`, `failure`, `unknown`).
- A redacted summary of the conversation structure.

You can revoke a trace contribution any time:

```sh
ironclaw traces revoke <submission-id> \
  --endpoint https://ingest.tracecommons.ai
```

(Find the `submission-id` via `ironclaw traces list-submissions`.)

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `400 InviteMalformed` from onboarding | Invite link is malformed or was copied with extra characters | Ask the operator to resend the invite link |
| `400 DeviceKeyMalformed` from onboarding | Ironclaw generated or encoded an invalid device public key | Upgrade Ironclaw and retry onboarding |
| `403 InviteNotValid` from onboarding | Invite hash is not allowlisted, revoked, or already consumed | Ask the operator to confirm the invite entry and `max_uses` |
| `503 OnboardRegistryNotConfigured` from onboarding | Issuer was deployed without the device-key registry DB | Operator needs to enable `TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED=true` |
| `401 missing bearer token` | Workload JWT expired | Refresh `IRONCLAW_TRACE_WORKLOAD_TOKEN` and rerun |
| `403 invite code not allowed` from issuer | Allowlist hasn't picked up your code yet (60s refresh) | Wait 60s, retry; if persistent, ask operator to confirm the hash landed |
| `403 consent scope ...` from issuer | The `--scope` you opted in with exceeds what the workload JWT permits | Re-run `opt-in` with a narrower `--scope`, or ask for a wider-scoped token |
| `400 trace contribution requires a pseudonymous contributor id` | Local pseudonymous ID generation failed for the scope | Re-run `opt-in` (it seeds the local ID); if still failing, report to operator |
| `500 trace commons operation failed` | Server-side error; logged hash-only on the server | Don't keep retrying. Send the operator your `pseudonymous_contributor_id` (from `traces status`) and the approximate timestamp |
| `status: quarantined`, `credit_points_pending: 0.0` for days | Body-carrying envelope waiting on privacy review (see [Accepted vs. quarantined outcomes](#accepted-vs-quarantined-outcomes)) | Either wait for the operator review pass, or drop `--include-message-text` / `--include-tool-payloads` on future submits to stay in the auto-accept lane |

## Asking for help

Ping the operator in the pilot channel with your
`pseudonymous_contributor_id` and the approximate timestamp of the
failure. Avoid pasting raw envelopes or workload JWTs into chat —
both are sensitive.

## Token rotation

The preferred onboarding path registers a local device key, so ordinary
participants should not need recurring workload JWT rotation. If you are
using the temporary fallback, workload JWTs are typically issued for 1h.
When yours is close to expiry, request a fresh one from the operator and
re-export `IRONCLAW_TRACE_WORKLOAD_TOKEN` (no need to re-run `opt-in` -
only the env value changes).

Registered device-key claim issuance is the default server path for the
pilot. Ironclaw signs claim requests with the private key it generated
locally during onboarding; the private key never leaves the machine.
