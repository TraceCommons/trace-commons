# Server-directed daily upload allowances

## Intent

Let the server raise a client's daily upload-count and byte allowances automatically, including when its queue is already blocked by a daily cap. The user selected automatic increases. Preserve explicit contributor choices, approvals, pause, and consent.

## Approaches

1. Return allowances only with uploads: small, but a capped client cannot discover an increase.
2. Publish an authenticated allowance that clients refresh independently of uploading (recommended). Reuse existing account HTTP/session handling; keep refresh independent of automatic-admission eligibility and its spent-allowance backoff.
3. Push notifications: adds a persistent transport and reconnection machinery for a change that tolerates periodic refresh.

Approach 2 was approved and implemented; the subsequent earned-trust refinement is recorded below.

## Contract and server control

Add an authenticated account endpoint returning an optional version-1 daily allowance: `max_uploads_per_day` and `max_bytes_per_day`. Put the wire type in the permissive protocol crate. Operator configuration supplies the two values together; absent configuration advertises no increase, and invalid or partial configuration fails startup. The operator values define a base daily allowance. Per-account increases derive from the same validated effective earned-trust allowance used by admission, covering both external and tiered policies. The user extended the feature to earned-trust-based increases after the initial implementation. Account binding and tenant authorization use existing route conventions. No new dependencies or database migration.

## Client behavior

Refresh at startup and every five minutes while signed in, independently of daily-cap state, automatic-project eligibility, and admission retry delays. Use the configured ingest origin, existing allowlisted transport, bounded request timeout, account-session rotation, and identity checks. Dry runs make no new requests. Sign-out or a change of account, tenant, or server discards the old allowance. A late reply cannot apply to a different identity.

Keep server allowance separate from persisted contributor settings. For each dimension without an explicit contributor override, the effective cap is the larger of the local default and the current valid server allowance. The existing hard ceilings remain 1,000 uploads and 5 GiB per day. Reject an invalid allowance as a whole. A failed refresh retains the last valid allowance for the current signed-in identity until restart; a successful response with no allowance clears it. Never reset usage counters when limits change. Upload enforcement and daemon budget/status must use the same effective caps.

Track explicit choices separately for upload count and bytes. Future local setting writes mark that dimension explicit, even if its value equals the default. For legacy settings without provenance, preserve nondefault values as explicit; default-valued legacy settings follow the server. Older files cannot distinguish a deliberately chosen default from an untouched default; document that migration limitation. Existing clients ignore the new mechanism and retain their current behavior.

## Verification

Regression coverage must show a capped queue resumes after a refreshed increase without changing today's counters; both dimensions enforce their effective cap; explicit choices survive refresh and restart; legacy settings migrate correctly; malformed, excessive, absent and failed responses behave as specified; identity changes and stale replies cannot reuse another account's allowance; and admission backoff cannot suppress allowance refresh. Exercise authenticated server configuration/route behavior, protocol compatibility, and daemon status consistency. Run focused tests, warning-denying compile/clippy checks, formatting, and the unchanged license-boundary test.

## Scope

No deployment, release, new consent, automatic approval of sessions, hard-ceiling increase, new trust scorer/thresholds, or per-account policy administration. Implementation details remain with the builder within the behavior above.

## Earned-trust refinement (2026-10-25)

User direction: users whose traces have been useful receive larger daily upload limits as trust grows. Reuse the existing applied trust/admission contract, including freshness, policy version, generation, shadow-mode exclusion, and fallback-to-base behavior. Do not infer trust from raw upload count, credit totals, a raw tier number, or a new public scoring rule.

Approaches considered: a new client-only quality score duplicates the trusted server evaluator; scaling a raw tier risks stale/shadow/private-policy inconsistencies; scaling from the validated applied admission allowance reuses the existing authority and works for both evaluator sources. Use the validated allowance.

For each bounded account, daily cap = floor(base daily cap * effective admission allowance / base admission allowance), clipped to the existing 1,000-upload/5-GiB hard ceilings. Integer arithmetic must tolerate i64-sized admission allowances without overflow. Configured client limits are base values; otherwise use the shipped 50-upload/200-MiB defaults when earned trust raises the entitlement. No earned increase and no configured base returns null, preserving default behavior and removing prior earned uplift when trust falls back. Invited and legacy accounts keep their existing independent paths without inferring an unbounded daily allowance.

Expose the validated effective admission allowance internally from the same database transaction as account status. Do not change the contribution-status wire body or expose evaluator policy internals. Resolve the account from authenticated tenant/principal context and verify account identity. Lookup failures refuse the endpoint; the existing client's transient-failure and identity-scope behavior remains unchanged.

Regression coverage: accounts with different validated entitlements receive different proportional caps; all policy invalidations fall to base; fixed operator settings act as a base; invitation bypass is preserved; counters/spend stay unchanged; policy absence retains existing behavior; hard ceilings and overflow remain bounded. No client protocol version change is needed.
