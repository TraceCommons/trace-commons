# Unbound passkey-account reaper

Z2 slice S5 (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`).
Cancel leaves an unbound passkey account inert; the reaper deletes it once it
has stayed `unbound` with no session activity for the TTL (default 30 days).
Bound, closed and legacy accounts (no binding row) are never touched.

## Enable

Off unless `TRACE_COMMONS_UNBOUND_REAPER_ENABLED=true`. When on, boot fails
closed without `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL`.

| Variable | Default | Bounds |
|---|---|---|
| `TRACE_COMMONS_UNBOUND_REAPER_ENABLED` | off | |
| `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL` | none, required when enabled | never the runtime URL |
| `TRACE_COMMONS_UNBOUND_REAPER_TTL_DAYS` | 30 | 1 to 3650 |
| `TRACE_COMMONS_UNBOUND_REAPER_INTERVAL_SECONDS` | 3600 | 60 to 86400 |
| `TRACE_COMMONS_UNBOUND_REAPER_BATCH_SIZE` | 100 | 1 to 1000 |

## Login

V99 creates two NOLOGIN roles. `trace_unbound_account_reaper_guard` owns the
`SECURITY DEFINER` function `trace_reap_unbound_accounts(BIGINT, INTEGER)` and
holds the column-scoped grants and permissive policies it needs.
`trace_unbound_account_reaper` holds EXECUTE and nothing else. Create a LOGIN
role that is unprivileged and inherits only the second, and point the URL at it:

```sql
CREATE ROLE tc_unbound_reaper_login LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '...';
GRANT trace_unbound_account_reaper TO tc_unbound_reaper_login;
```

A superuser or BYPASSRLS login is not needed and defeats the point.

## Evidence

Each tick logs `reaped` and `skipped` counts and the TTL; nothing else leaves
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
