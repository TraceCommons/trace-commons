# Server-directed daily upload allowances

## Intent

Let the server raise a client's daily upload-count and byte allowances automatically, including when its queue is already blocked by a daily cap. The user selected automatic increases. Preserve explicit contributor choices, approvals, pause, and consent.

## Approaches

1. Return allowances only with uploads: small, but a capped client cannot discover an increase.
2. Publish an authenticated allowance that clients refresh independently of uploading (recommended). Reuse existing account HTTP/session handling; keep refresh independent of automatic-admission eligibility and its spent-allowance backoff.
3. Push notifications: adds a persistent transport and reconnection machinery for a change that tolerates periodic refresh.

Proceed with approach 2, subject to design approval.

## Contract and server control

Add an authenticated account endpoint returning an optional version-1 daily allowance: `max_uploads_per_day` and `max_bytes_per_day`. Put the wire type in the permissive protocol crate. Operator configuration supplies the two values together; absent configuration advertises no increase, and invalid or partial configuration fails startup. This first slice uses one configured allowance for authenticated accounts; account-specific tuning is out of scope. Account binding and tenant authorization use existing route conventions. No new dependencies or database migration.

## Client behavior

Refresh at startup and every five minutes while signed in, independently of daily-cap state, automatic-project eligibility, and admission retry delays. Use the configured ingest origin, existing allowlisted transport, bounded request timeout, account-session rotation, and identity checks. Dry runs make no new requests. Sign-out or a change of account, tenant, or server discards the old allowance. A late reply cannot apply to a different identity.

Keep server allowance separate from persisted contributor settings. For each dimension without an explicit contributor override, the effective cap is the larger of the local default and the current valid server allowance. The existing hard ceilings remain 1,000 uploads and 5 GiB per day. Reject an invalid allowance as a whole. A failed refresh retains the last valid allowance for the current signed-in identity until restart; a successful response with no allowance clears it. Never reset usage counters when limits change. Upload enforcement and daemon budget/status must use the same effective caps.

Track explicit choices separately for upload count and bytes. Future local setting writes mark that dimension explicit, even if its value equals the default. For legacy settings without provenance, preserve nondefault values as explicit; default-valued legacy settings follow the server. Older files cannot distinguish a deliberately chosen default from an untouched default; document that migration limitation. Existing clients ignore the new mechanism and retain their current behavior.

## Verification

Regression coverage must show a capped queue resumes after a refreshed increase without changing today's counters; both dimensions enforce their effective cap; explicit choices survive refresh and restart; legacy settings migrate correctly; malformed, excessive, absent and failed responses behave as specified; identity changes and stale replies cannot reuse another account's allowance; and admission backoff cannot suppress allowance refresh. Exercise authenticated server configuration/route behavior, protocol compatibility, and daemon status consistency. Run focused tests, warning-denying compile/clippy checks, formatting, and the unchanged license-boundary test.

## Scope

No deployment, release, new consent, automatic approval of sessions, hard-ceiling increase, or per-account policy administration. Implementation details remain with the builder within the behavior above.
