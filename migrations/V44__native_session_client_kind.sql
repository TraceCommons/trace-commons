-- V44: admit 'native' as a session client_kind.
--
-- The loopback native-app sign-in flow mints a bearer token backed by an
-- ordinary `trace_sessions` row, so that expiry, the idle cap,
-- rotation-on-use, and `POST /v1/account/sessions/revoke-all` all govern it
-- with no second code path. That row needs a `client_kind` of its own: 'web'
-- would misreport a native client as a browser, and 'device' would conflate a
-- session with the device upload credential.
--
-- 'native' is deliberately NOT added to the strong-authenticator set
-- (`AccountCtx::is_strong_session`, which admits only 'passkey' and 'near'):
-- a native token stays weak, so it can read and withdraw but can never change
-- an account's authenticators or redirect a payout.
--
-- No new table, so no new RLS policy: `trace_sessions` already has forced RLS
-- from V30 and this migration does not touch it.
--
-- V30 declared the constraint inline, so the system-generated name is
-- trace_sessions_client_kind_check (same as V32 and V33 relied on).

ALTER TABLE trace_sessions
    DROP CONSTRAINT trace_sessions_client_kind_check,
    ADD CONSTRAINT trace_sessions_client_kind_check
        CHECK (client_kind IN ('web', 'device', 'passkey', 'near', 'native'));
