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

The operator hands you four things over a secure channel (1Password,
Signal, etc. — not email or Slack DMs):

1. **Invite code.** 16 chars, e.g. `FMJD93VFPROQ5I4I`. The issuer
   hashes this with sha256 and checks against its allowlist; the raw
   code never lands in the DB.
2. **Workload JWT** — a short-lived EdDSA JWT (typically 1h) the
   operator minted with `sign-workload-token.py`. Ironclaw bearers it
   to `/v1/trace-upload-claim`; the issuer then mints a separate
   upload claim that ingest accepts. When yours expires, ask for a
   fresh one (or move to per-contributor signing keys; see [Token
   rotation](#token-rotation)).
3. **Tenant ID.** All pilot contributors currently share
   `tenant-zaki-pilot`. Per-contributor tenants are deferred until the
   cohort grows.
4. **Endpoint URLs**:
   - Ingest: `https://ingest.34-41-15-28.nip.io`
   - Issuer: `https://issuer.34-41-15-28.nip.io`

## Prerequisites

- Ironclaw built from a recent `main` (PR #3738 or later). Confirm
  with `ironclaw --version`.
- Whatever LLM provider config you already use with Ironclaw — the
  trace pipeline is independent of inference, it just observes what
  your agent did.

## Configure Ironclaw

Trace contribution is a local-first, opt-in feature. There's no TOML
config block to edit; you configure it once with `ironclaw traces
opt-in`, which persists a standing policy under
`~/.ironclaw/trace_contributions/`. The only env var the runtime
reads at submission time is the one holding your workload JWT.

Export the workload JWT first:

```sh
export IRONCLAW_TRACE_WORKLOAD_TOKEN='<jwt-from-operator>'
```

Then opt in:

```sh
ironclaw traces opt-in \
  --endpoint https://ingest.34-41-15-28.nip.io \
  --upload-token-issuer-url https://issuer.34-41-15-28.nip.io \
  --upload-token-issuer-allowed-hosts issuer.34-41-15-28.nip.io \
  --upload-token-audience trace-commons-ingest \
  --upload-token-tenant-id tenant-zaki-pilot \
  --upload-token-workload-token-env IRONCLAW_TRACE_WORKLOAD_TOKEN \
  --upload-token-invite-code '<invite-code-from-operator>' \
  --scope debugging-evaluation
```

Notes:
- `--upload-token-issuer-allowed-hosts` is a hostname allowlist Ironclaw
  enforces on the issuer URL — protects against misconfigured DNS.
- `--upload-token-invite-code` is required because the pilot issuer runs
  with `TRACE_COMMONS_ALLOWLIST_SOURCE` set.
- `--scope` controls which uses your envelopes consent to.
  `debugging-evaluation` covers debugging + evaluation + aggregate
  analytics (matches what the operator mints into your workload JWT).
- Don't pass `--include-message-text` or `--include-tool-payloads`
  unless you've explicitly decided to share them — both stay off by
  default and the pilot accepts envelopes either way.
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

A successful submit prints:

```
{"status":"accepted","credit_points_pending":5.2,"explanation":["Accepted into the private redacted corpus.","Attributed to tenant tenant_sha256:..."]}
```

Or `credit_points_pending: 0.0` if you've already submitted that exact
content (the gate suppresses duplicates).

Check what landed:

```sh
ironclaw traces list-submissions
ironclaw traces credit
ironclaw traces queue-status
```

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
  --endpoint https://ingest.34-41-15-28.nip.io
```

(Find the `submission-id` via `ironclaw traces list-submissions`.)

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `401 missing bearer token` | Workload JWT expired | Refresh `IRONCLAW_TRACE_WORKLOAD_TOKEN` and rerun |
| `403 invite code not allowed` from issuer | Allowlist hasn't picked up your code yet (60s refresh) | Wait 60s, retry; if persistent, ask operator to confirm the hash landed |
| `403 consent scope ...` from issuer | The `--scope` you opted in with exceeds what the workload JWT permits | Re-run `opt-in` with a narrower `--scope`, or ask for a wider-scoped token |
| `400 trace contribution requires a pseudonymous contributor id` | Local pseudonymous ID generation failed for the scope | Re-run `opt-in` (it seeds the local ID); if still failing, report to operator |
| `500 trace commons operation failed` | Server-side error; logged hash-only on the server | Don't keep retrying. Send the operator your `pseudonymous_contributor_id` (from `traces status`) and the approximate timestamp |

## Asking for help

Ping the operator in the pilot channel with your
`pseudonymous_contributor_id` and the approximate timestamp of the
failure. Avoid pasting raw envelopes or workload JWTs into chat —
both are sensitive.

## Token rotation

Workload JWTs are typically issued for 1h. When yours is close to
expiry, request a fresh one from the operator and re-export
`IRONCLAW_TRACE_WORKLOAD_TOKEN` (no need to re-run `opt-in` — only
the env value changes).

If the cohort grows beyond a handful of devs, the pilot will move
from operator-mints-on-demand to per-contributor signing keys (each
contributor's pubkey loaded into the issuer's allowed-workload-key
set); your client would then sign its own JWTs from a private key
you hold locally. That path requires a small issuer change (the
issuer today loads a single `_WORKLOAD_PUBLIC_KEY_FILE`) and is
tracked as a pilot follow-up.
