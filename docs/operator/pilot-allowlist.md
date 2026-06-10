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
the current allowlist snapshot. The same snapshot powers
`POST /v1/onboard`, where an Ironclaw agent exchanges an invite code plus
device public key for tenant-scoped onboarding metadata and a registered
device key.

Upload-claim refusals use four explicit error labels that operators can
grep for in client error logs:

| Label | HTTP | Meaning |
|---|---|---|
| `PilotAllowlistInviteCodeMissing` | 400 | Workload claims have no `invite_code` field |
| `PilotAllowlistNotMatched` | 403 | Hash of the supplied invite code is not in the snapshot |
| `PilotAllowlistStale` | 503 | Cached snapshot is older than `max_stale_seconds` and the source has not reloaded successfully |
| `PilotAllowlistMalformed` | 503 | Source returned a parse failure and no cached snapshot is available to fall back to |

No raw invite codes or contributor identities appear in any log line,
audit row, or admin response.

Onboarding refusals use these public labels:

| Label | HTTP | Meaning |
|---|---|---|
| `InviteMalformed` | 400 | Invite schema/version/format is invalid |
| `DeviceKeyMalformed` | 400 | Device public key is not base64 Ed25519 public-key bytes |
| `InviteNotValid` | 403 | Invite hash is not allowlisted, revoked, or out of uses |
| `OnboardAllowlistNotConfigured` | 503 | Issuer has no allowlist source |
| `OnboardRegistryNotConfigured` | 503 | Device-key registry DB is not enabled |
| `OnboardTenantConfigMissing` | 503 | Issuer is missing the onboarding URL config returned to clients |
| `OnboardAllowlistStale` | 503 | Cached snapshot is stale and the source has not reloaded successfully |

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
      "note_label": "closed-alpha-batch-1",
      "max_uses": 1
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
- `note_label`: optional pseudonymous/batch label. It is returned as
  `contributor_label` from `/v1/onboard` and never appears in logs or
  admin responses. Do not put a legal name, email, account id, Slack
  handle, or any identifying reference here.
- `max_uses`: positive integer. Defaults to `1` when omitted. The
  workload-claim path treats this as metadata, while `/v1/onboard`
  enforces it through the PostgreSQL `onboarding_invites` counter.

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

For agent-driven onboarding, also enable the device-key registry and
return URLs:

```bash
export TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED=true
export DATABASE_URL=postgres://app:<password>@127.0.0.1:5432/trace-commons
export TRACE_COMMONS_ONBOARDING_INGEST_URL=https://ingest.tracecommons.ai
export TRACE_COMMONS_ONBOARDING_COMMUNITY_URL=https://tracecommons.ai
export TRACE_COMMONS_ONBOARDING_PROFILE_URL=https://tracecommons.ai/profile
export TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL=https://tracecommons.ai/leaderboard
```

`TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED=true` makes the
issuer connect to PostgreSQL at startup, run migrations through the
shared Trace Commons DB path, and fail closed if the registry is not
available.

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

## Agent-driven onboarding smoke

After the allowlist and registry env are live, Ironclaw can exchange the
invite for a tenant/device registration:

```bash
curl -sS https://issuer.tracecommons.ai/v1/onboard \
  -H 'Content-Type: application/json' \
  -d '{
    "schema_version":"trace_commons.onboard_request.v1",
    "invite_code":"INV9K3RT5FBQ72JX",
    "device_public_key":"<base64-ed25519-public-key>",
    "client_info":{"agent":"ironclaw","version":"<version>"}
  }' | jq
```

Expected success shape:

```json
{
  "schema_version": "trace_commons.onboard_response.v1",
  "tenant_id": "tenant-zaki-pilot",
  "ingest_url": "https://ingest.tracecommons.ai",
  "issuer_url": "https://issuer.tracecommons.ai",
  "audience": "trace-commons-ingest",
  "device_key_id": "sha256:<64-hex>",
  "contributor_label": "closed-alpha-batch-1",
  "community_url": "https://tracecommons.ai",
  "profile_url": "https://tracecommons.ai/profile",
  "leaderboard_url": "https://tracecommons.ai/leaderboard"
}
```

The response is safe for Ironclaw to store in its local contribution
profile. It contains only tenant/config labels and hash-derived device
identity. It does not return the raw invite code, bearer tokens, or
operator identity.

## Managing registered device keys

Agent-driven onboarding stores one post-invite device key per Ironclaw
agent in the PostgreSQL `device_keys` registry. The admin surface is
tenant-scoped and RLS-enforced; list and revoke operations use the
authenticated tenant from the operator token. The optional `--tenant`
flag is a guardrail and must match that authenticated tenant.

List active device keys:

```bash
trace-commons-tenant \
  --endpoint https://ingest.tracecommons.ai \
  device-keys list \
  --tenant tenant-zaki-pilot
```

Include revoked keys:

```bash
trace-commons-tenant \
  --endpoint https://ingest.tracecommons.ai \
  device-keys list \
  --tenant tenant-zaki-pilot \
  --include-revoked
```

Revoke one device key:

```bash
trace-commons-tenant \
  --endpoint https://ingest.tracecommons.ai \
  device-keys revoke sha256:<64-hex-device-key-id>
```

The response surfaces `device_key_id`, `invite_subject_hash`,
`client_info`, and timestamps. It does not return raw invite codes,
bearer tokens, contributor identities, or trace content.

## Repairing profile setup after onboarding

If an onboarded Ironclaw agent says the operator needs to check invite
device-key status before it can set a public profile, verify the device
key is registered and grant that device principal profile-management
scope.

First list active device keys and confirm the participant's
`device_key_id` is present:

```bash
trace-commons-tenant \
  --endpoint https://ingest.tracecommons.ai \
  device-keys list \
  --tenant tenant-zaki-pilot
```

Then derive the tenant access-grant principal for that device key:

```bash
trace-commons-tenant tenant-principal-ref \
  --device-tenant-id tenant-zaki-pilot \
  --device-key-id sha256:<64-hex-device-key-id>
```

Grant the device contributor access with both normal pilot trace scopes
and the separate public-profile scope:

```bash
trace-commons-tenant \
  --endpoint https://ingest.tracecommons.ai \
  tenant-access-grant-create \
  --principal-ref principal_sha256:<64-hex-principal-ref> \
  --role contributor \
  --allowed-consent-scopes debugging-evaluation,public-attribution \
  --allowed-uses debugging,evaluation,aggregate-analytics \
  --reason "grant pilot device-key trace and profile access"
```

The `principal_ref` is already device-specific. Only add optional
`--issuer`, `--audience`, or `--subject` filters when you are certain they
match the live issuer claim values; a mismatch will make claim issuance
fail closed.

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
- The workload-token `/v1/trace-upload-claim` path still treats file
  invite codes as admission checks. Atomic `max_uses` enforcement is on
  the agent-driven `/v1/onboard` path through PostgreSQL
  `onboarding_invites`.
- Admin auth is loopback-only by default. If you need to expose
  `/v1/admin/allowlist-status` over a tunnel or behind an internal
  bearer-gated reverse proxy, set
  `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` and put the bearer in
  the proxy.
- Denial counter is process-local. Restart resets it. It's a
  "rough current pressure" signal, not an audit surface.
