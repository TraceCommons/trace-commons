# Pilot Allowlist — Operator Runbook

Gate the upload-claim issuer to "Ironclaw contributors only" for the
pilot. The allowlist lives on the issuer process; the ingest server's
existing tenant / role / grant model is unchanged.

Spec: `docs/superpowers/specs/2026-05-17-pilot-allowlist-design.md`.
Implementation plan: `docs/superpowers/plans/2026-05-17-pilot-allowlist.md`.

## What this gate does

When the issuer is configured with `TRACE_COMMONS_ALLOWLIST_SOURCE`, the
`POST /v1/trace-upload-claim` handler refuses any workload token that
either omits `invite_code` or carries an invite code whose hash is not in
the current allowlist snapshot. Refusals use four explicit error labels
that operators can grep for in client error logs:

| Label | HTTP | Meaning |
|---|---|---|
| `PilotAllowlistInviteCodeMissing` | 400 | Workload claims have no `invite_code` field |
| `PilotAllowlistNotMatched` | 403 | Hash of the supplied invite code is not in the snapshot |
| `PilotAllowlistStale` | 503 | Cached snapshot is older than `max_stale_seconds` and the source has not reloaded successfully |
| `PilotAllowlistMalformed` | 503 | Source returned a parse failure and no cached snapshot is available to fall back to |

No raw invite codes or contributor identities appear in any log line,
audit row, or admin response.

## Provisioning an invite code

Two steps: pick a code, hash it, append to the allowlist JSON.

### 1. Pick a code

Anything operator-meaningful works — the contributor pastes it back via
their workload-token signer. Suggest 16 chars `[A-Z0-9]`, no leading
zero, no ambiguous glyphs:

```bash
LC_ALL=C tr -dc 'A-Z2-9' </dev/urandom | head -c 16
```

Example: `INV9K3RT5FBQ72JX`.

### 2. Hash it

Use the issuer's own helper so the hashing function never drifts from
the issuance handler:

```bash
trace-commons-upload-claim-issuer --hash-invite-code INV9K3RT5FBQ72JX
# → sha256:8b1a... (64 hex chars)
```

### 3. Append to the allowlist JSON

```json
{
  "version": 1,
  "generated_at": "2026-05-17T18:00:00Z",
  "policy_label": "pilot-2026-05",
  "entries": [
    {
      "subject_hash": "sha256:8b1a...",
      "tenant_id": "tenant-zaki-pilot",
      "note_label": "closed-alpha-batch-1"
    }
  ]
}
```

- `version`: must be 1. Anything else is rejected as
  `PilotAllowlistMalformed`.
- `policy_label`: appears in minted JWTs as the `policy_label` claim and
  in `/v1/admin/allowlist-status` responses. Use it to mark the batch
  ("pilot-2026-05", "closed-alpha-q3").
- `subject_hash`: must be lowercase canonical `sha256:<64 hex>`. The
  schema rejects uppercase hex on purpose so the operator notices if
  they generated the hash with a different tool.
- `tenant_id`: the tenant the contributor will be attributed to. The
  issuer does not currently force tenant equality against this field —
  the existing workload-claim flow already resolves the minted tenant —
  but it's stored for future cross-checks and operator-side auditing of
  "who is allowed where".
- `note_label`: operator-facing free text. Never returned to clients,
  never logged.

The issuer re-reads the file every
`TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS` (default 60), so a
file edit takes effect within a minute without a restart.

## Running the issuer with the allowlist enabled

Required env:

```bash
export TRACE_COMMONS_ALLOWLIST_SOURCE=file:/etc/tracecommons/allowlist.json
```

Optional env (defaults shown):

```bash
export TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS=60
export TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS=3600
export TRACE_COMMONS_ISSUER_ADMIN_BIND=127.0.0.1:3918   # see "Admin endpoint"
```

The issuer warms the source eagerly during startup; a missing or
malformed file aborts the process with
`PilotAllowlistSourceMissing` / `PilotAllowlistMalformed` rather than
silently letting traffic through. Same fail-closed posture as the rest
of the gate stack.

## Reading `/v1/admin/allowlist-status`

Only mounted when `TRACE_COMMONS_ISSUER_ADMIN_BIND` is set. The bind
address MUST be loopback (`127.0.0.1` / `::1`) unless
`TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` is also set; the
startup guard refuses a non-loopback bind otherwise with
`PilotAllowlistAdminBindNotLoopback`.

```bash
$ curl -s http://127.0.0.1:3918/v1/admin/allowlist-status | jq
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

Fields:

- `configured`: `false` when no allowlist source is set; the response
  collapses to just `{ "configured": false }` in that case.
- `entries`: count only. Subject hashes, tenant ids, and note labels
  are never returned.
- `snapshot_age_seconds`: time since the source was last successfully
  reloaded. Bumps back to ~0 after each refresh.
- `denials_last_hour`: sliding-window count of `PilotAllowlistNotMatched`
  refusals. Process-local; restart resets.
- `stale`: `true` when `snapshot_age > max_stale_seconds`. Once stale,
  issuance refuses with 503 `PilotAllowlistStale` until the source
  reloads.

If the response status is 503 (instead of 200), the source is failing
to reload and the cached snapshot is exhausted — fix the source
immediately or expect issuance refusals.

## Denial smoke

After enabling the allowlist, prove the gate fires:

1. Pick a code that is NOT in the file (e.g., `INV-NOT-IN-PILOT`).
2. Sign a workload token with `invite_code: "INV-NOT-IN-PILOT"`.
3. POST it to `/v1/trace-upload-claim`. Expect:
   ```
   HTTP/1.1 403 Forbidden
   {"error":"PilotAllowlistNotMatched"}
   ```
4. Then post a workload token whose `invite_code` IS in the file.
   Expect 200 with `access_token`. Decode the JWT body and verify the
   `policy_label` claim matches the file's `policy_label`.

If step 3 returns 200, the allowlist source is not wired up — confirm
`TRACE_COMMONS_ALLOWLIST_SOURCE` is set in the issuer's environment
(it must be set at the same scope as the other
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_*` env vars).

## Adding a contributor mid-pilot

1. Send the contributor a fresh invite code through whatever recruitment
   channel the pilot uses (form, DM, signed announcement).
2. Hash it with `--hash-invite-code`.
3. Append a new entry to the allowlist JSON.
4. Wait up to `TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS` (default
   60s) — no restart, no redeploy.
5. Confirm `entries` in `/v1/admin/allowlist-status` ticked up by one.

## Rollback

To turn the gate off:

1. Unset `TRACE_COMMONS_ALLOWLIST_SOURCE` from the issuer's environment.
2. Restart the issuer.
3. The issuer reverts to the pre-allowlist MVP: any workload token with
   a valid signature is admitted, regardless of `invite_code`. No
   schema migration, no data cleanup.

## Coordination with the workload-token signer

The Ironclaw client and the operator-side workload-token signer both
need to start populating `invite_code` before any allowlisted
contributor's first refresh succeeds. Workload tokens without the field
keep parsing fine (the field is `Option<String>` with `#[serde(default)]`)
but get refused at the allowlist check with
`PilotAllowlistInviteCodeMissing` (400). That's the easiest failure to
diagnose — clear refusal class, exactly the right thing to grep for.

## Known limitations

- File source only. The `near:<account>:<view>` source is reserved in
  the CLI surface but rejected at construction with
  `PilotAllowlistNearSourceNotImplemented`. The on-chain allowlist
  source ships in a later slice once the closed-alpha operational story
  is in.
- Admin auth is loopback-only by default. If you need to expose
  `/v1/admin/allowlist-status` over a tunnel or behind an internal
  bearer-gated reverse proxy, set
  `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` and put the bearer in
  the proxy.
- Denial counter is process-local. Restart resets it. It's a
  "rough current pressure" signal, not an audit surface.
