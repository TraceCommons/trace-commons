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
| `InviteNotValid` | 403 | Invite hash is not allowlisted or revoked |
| `InviteAlreadyConsumed` | 403 | Invite is valid but its retry budget is exhausted |
| `OnboardAllowlistNotConfigured` | 503 | Issuer has no allowlist source |
| `OnboardRegistryNotConfigured` | 503 | Device-key registry DB is not enabled |
| `OnboardTenantConfigMissing` | 503 | Issuer is missing the onboarding URL config returned to clients |
| `OnboardAllowlistStale` | 503 | Cached snapshot is stale and the source has not reloaded successfully |
| `InviteExpired` | 403 | Reserved wire label for an invite that failed only because it expired. The registry-backed redemption path does not currently distinguish this from `InviteNotValid` — expired, revoked, and never-existed invites all collapse to the same 403 today. |
| `InviteRegistryNotConfigured` | 503 | Authoritative mode is on (`TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE=true`) but `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` is not set, so there is no registry to redeem against |
| `InviteRegistryStale` | 503 | Authoritative mode is on and the registry's in-process cache has not refreshed within its staleness window |

## Provisioning invite codes

Invites live in PostgreSQL. The allowlist file no longer carries them; it
keeps only `kind: "instance"` TEE entries and the `policy_label`.

### One-time role provisioning

The registry pool runs as a narrow role that cannot bypass RLS. Migration
`V42` creates the base `trace_invite_registry` role (`NOLOGIN`,
`NOBYPASSRLS`, permissive row policy scoped to that role); provision a
login role and grant it membership:

```sql
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_invite_registry_login') THEN
        CREATE ROLE trace_invite_registry_login LOGIN PASSWORD '<generated>' NOBYPASSRLS;
    END IF;
END $$;
GRANT trace_invite_registry TO trace_invite_registry_login;
```

Point `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` at that login role.
Without it, the admin invite routes (`/v1/admin/invites*`,
`/v1/admin/invite-registry-status`) do not mount at all, and redemption
under authoritative mode fails closed with `InviteRegistryNotConfigured`.
Once that variable IS set, an unreachable database aborts issuer startup —
the same fail-closed posture as every other `configure_*_from_env` gate in
this issuer, not a bug. Do not set it against a database you have not
provisioned.

### Minting a batch

```bash
trace-commons-upload-claim-issuer --mint-invites 5 \
  --policy-label pilot-2026-08 \
  --mint-tenant-template pilot-2026-08 \
  --mint-max-uses 3 \
  --mint-expires-in-days 30 \
  > /tmp/tracecommons-invite-codes.txt
```

Each line is one raw code. It is never stored and cannot be recovered: only
its hash reaches the database. Delete the file after handing the codes out.
This connects directly to the database via `DATABASE_URL` and
`TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` — no running issuer or admin
token required, so an operator can mint before the issuer is even up.

No issuer restart is needed, but codes minted this way are NOT immediately
redeemable: `--mint-invites` writes to the database directly and the
running issuer's cache only picks it up on its next refresh
(`TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS`, default 60). An
operator who mints via the CLI and immediately tests redemption will see it
fail for up to one refresh interval — that is expected, not a bug. Minting
through the admin API instead (`POST /v1/admin/invites`, below) is
redeemable immediately, because the handler invalidates the in-process
cache on write, in the same request that committed the insert.

```bash
curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -H "Content-Type: application/json" \
  -d '{"tenant_mode":"derived","tenant_template_id":"pilot-2026-08","max_uses":3}' \
  "http://127.0.0.1:3918/v1/admin/invites"
```

### Revoking

```bash
curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_JWT" \
  "http://127.0.0.1:3918/v1/admin/invites/$INVITE_HASH/revoke"
```

Revocation takes effect immediately: the database refuses the redemption and
the cache entry is dropped in the same request.

### Checking registry health

```bash
curl -sS -H "Authorization: Bearer $ADMIN_JWT" \
  "http://127.0.0.1:3918/v1/admin/invite-registry-status"
```

`stale: true` means the cache has not reloaded within `max_stale_seconds`
and redemption is failing closed with `InviteRegistryStale`.

## Manual single-invite flow

For hashing a code you already have (e.g. one an operator generated by
hand, or when cross-checking a `--mint-invites` output), use the issuer's
own helper so the hashing function never drifts from the issuance handler:

```bash
trace-commons-upload-claim-issuer --hash-invite-code INV9K3RT5FBQ72JX
# → sha256:8b1a... (64 hex chars)
```

There is no manual JSON-editing path anymore once the invite registry is
authoritative — see "Cutover" below. Mint through `--mint-invites` or
`POST /v1/admin/invites` instead.

## Cutover

Making PostgreSQL authoritative for invites is a four-step rollout, so a
mid-flight operator error never strands contributors on either side of the
switch:

1. **Ship with the file still authoritative.** The database-backed
   redemption path and the admin routes exist and can be exercised, but
   `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE` is unset, so `/v1/onboard`
   keeps redeeming against the allowlist file exactly as before.
2. **Run `--import-file-invites`.** One-time migration of the file's
   existing `kind: "invite"` entries into the database. Idempotent on the
   invite hash, so a partial or repeated run is safe. Instance entries stay
   in the file and are counted, not imported.
3. **Set the authoritative flag and strip invite entries from the file.**
   Set `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE=true` and remove every
   `kind: "invite"` entry from the allowlist JSON (instance entries stay).
   At this point the file entries are inert either way — the database
   decides — but leaving them in is a rollback safety net (next point).
4. **The following release makes invite entries a parse error.** As of this
   release, once operators have confirmed step 3 is clean, invite entries
   left in the allowlist file are a hard `PilotAllowlistMalformed`
   startup/reload failure rather than a silent no-op. This is deliberate:
   once the database is authoritative, a stale file entry could otherwise
   re-authorize an invite that was revoked in the database.

Rollback between steps 3 and 4 is just unsetting
`TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE` — the file's invite entries
are still present (you have not reached step 4 yet) and `--import-file-invites`
is idempotent, so nothing needs to be replayed. There is no rollback once
step 4 has shipped except re-adding entries to the file and deploying an
older issuer build; this release refuses them outright.

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

1. Mint one invite (or a small batch) through `--mint-invites` or
   `POST /v1/admin/invites` — see "Provisioning invite codes" above. Prefer
   the admin API if you plan to hand the code out immediately: it is
   redeemable the moment the request returns, with no refresh-interval wait.
2. Confirm `GET /v1/admin/invite-registry-status` reports `stale: false`.
3. Send the contributor the fresh invite code through whatever recruitment
   channel the pilot uses (form, DM, signed announcement).

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

## Repairing legacy profile setup after onboarding

Current `/v1/onboard` registrations create a default contributor tenant
access grant for the device key. That grant includes the normal pilot trace
scopes plus the separate `public_attribution` profile-management scope, so
operators should not need to hand-grant profile access for new invites.

Use this repair flow only for devices onboarded before the default grant was
created automatically, or for a device whose default grant was removed by an
operator.

If an onboarded Ironclaw agent says the operator needs to check invite
device-key status before it can set a public profile, verify the device key is
registered and grant that device principal profile-management scope.

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

## Reading enrollment grants for per-user consent scopes

Instance-vouched enrollment writes a per-device tenant-access-grant row
carrying the instance policy template's `allowed_consent_scopes` /
`allowed_uses`. For a device-key upload claim to honor those broadened
scopes (e.g. `model_training`), the issuer must READ that grant at claim
time. Two env flags control this, decoupled:

- `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB=1` — attach the
  grant DB for READING only. Device-key claims derive their consent-scope
  ceiling from the enrollment grant; no strict enforcement is imposed on
  any other path. This is the flag to set for per-user consent (Devfolio).
- `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS=1` —
  attach the grant DB AND require an active grant for every claim (strict).
  Implies the read behavior above. Do not enable on the pilot yet: the
  ingest-side grant-principal alignment for grant-required deployments is a
  separate, unlanded slice.

With neither flag set, device-key claims fall back to the hardcoded floor
`[debugging_evaluation, public_attribution]` and any requested
`model_training` scope is clipped out.
