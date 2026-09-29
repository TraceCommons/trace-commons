# Unbound passkey-account reaper

Z2 slice S5 (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`).
Cancel leaves an unbound passkey account inert; the reaper deletes it once it
has stayed `unbound` and idle past one of two windows. Bound, closed and legacy
accounts (no binding row) are never touched.

## The two windows

| Account | Window (default) | Variable |
|---|---|---|
| Never used again after creation | 7 days from creation | `TRACE_COMMONS_UNBOUND_REAPER_NEVER_USED_TTL_DAYS` |
| Signed in again after creation | 30 days since the last session activity | `TRACE_COMMONS_UNBOUND_REAPER_TTL_DAYS` |

An account with an unrevoked, unexpired session is never reaped, whichever
window applies. The never-used window may not exceed the idle window: an
explicit value above it refuses boot, and V99's function refuses the call. If
only `TTL_DAYS` is lowered below 7, the never-used default follows it down.

### What "never used again" means

Native account creation (S2, `create/finish`) writes, in one transaction, the
binding row and exactly one native session, both stamped with that
transaction's `now()` (`created_at = last_seen_at`), the session expiring after
12 hours. An account counts as used again when EITHER

1. it has more than one session row (a later sign-in mints a new row; sessions
   are revoked, never deleted), OR
2. some session's `last_seen_at` is more than one hour after the binding's
   `created_at` (the creation session was presented again later).

The one-hour grace lets the app present the creation session in the minutes
after creation (bind start, step-up) without promoting the account to the long
window. It is a constant in V99, not configuration. Anything else is never
used again. The rule reads only `trace_sessions.last_seen_at` and the row count,
not session `created_at`. It is re-checked under the row locks, so a sign-in
that commits during a sweep moves the account to the 30-day window and it
survives.

## Enable

Off unless `TRACE_COMMONS_UNBOUND_REAPER_ENABLED=true`. When on, boot fails
closed without `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL`.

| Variable | Default | Bounds |
|---|---|---|
| `TRACE_COMMONS_UNBOUND_REAPER_ENABLED` | off | |
| `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL` | none, required when enabled | never the runtime URL |
| `TRACE_COMMONS_UNBOUND_REAPER_TTL_DAYS` | 30 | 1 to 3650 |
| `TRACE_COMMONS_UNBOUND_REAPER_NEVER_USED_TTL_DAYS` | 7 | 1 to 3650, at most `TTL_DAYS` |
| `TRACE_COMMONS_UNBOUND_REAPER_INTERVAL_SECONDS` | 3600 | 60 to 86400 |
| `TRACE_COMMONS_UNBOUND_REAPER_BATCH_SIZE` | 100 | 1 to 1000 |

## Login

V99 creates two NOLOGIN roles. `trace_unbound_account_reaper_guard` owns the
`SECURITY DEFINER` function `trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER)` and
holds the column-scoped grants and permissive policies it needs.
`trace_unbound_account_reaper` holds EXECUTE and nothing else. Create a LOGIN
role that is unprivileged and inherits only the second, and point the URL at it:

```sql
CREATE ROLE tc_unbound_reaper_login LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '...';
GRANT trace_unbound_account_reaper TO tc_unbound_reaper_login;
```

A superuser or BYPASSRLS login is not needed and defeats the point.

## Evidence

Each tick logs `reaped` and `skipped` counts and both TTLs; nothing else leaves
the database. The loop reports through the driver liveness registry as
`unbound_account_reaper`. `skipped` counts candidates that were locked by a
concurrent bind or sign-in, active again, or refused by a foreign key; a
`skipped` that never falls to zero means an account holds rows the cascade
cannot remove and needs a look. A skipped candidate does not use up the
batch: each call deletes up to `limit` accounts and examines at most
`10 * limit` candidates, stepping past skipped ones, so a few permanently
refused accounts do not starve the rest. A refused backlog larger than that
scan cap would, which is another reason a persistent `skipped` needs a look. `trace_audit_events` rows for a reaped tenant
are retained (they are hash-chained and have no tenant foreign key).
