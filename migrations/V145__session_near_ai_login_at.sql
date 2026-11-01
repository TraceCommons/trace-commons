-- When a NEAR AI sign-in minted this session.
--
-- The strong-authenticator gate lets a session change authenticators (a NEAR
-- key added or removed), the payout target, or merge accounts only if it is a
-- NEAR wallet session or a NEAR AI sign-in made within the last few minutes
-- (`NEAR_AI_LOGIN_FRESH_SECS`). Nothing on a session row told a NEAR AI
-- sign-in from a device's own renewal: both insert the same columns. The
-- sign-in now stamps this column; a renewal, a device-link sign-in and the
-- loopback native sign-in leave it NULL, and rotation-on-use never changes it.
--
-- Additive and nullable, with no backfill: a session minted before this
-- migration reads as no recent NEAR AI sign-in, which only ever refuses.
-- trace_sessions was granted to the ingest runtime table-wide, which covers a
-- new column.

ALTER TABLE trace_sessions ADD COLUMN IF NOT EXISTS near_ai_login_at TIMESTAMPTZ;
