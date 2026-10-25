# Client daily upload allowance

The server advertises daily upload-count and byte limits for the authenticated account. An account's validated earned admission allowance can raise both daily caps automatically. These client volume limits do not approve sessions or change consent or pause. Server admission still applies its separate processing-cost units and accounting period; a daily cap does not establish that an account is ready to submit or has admission capacity remaining.

## Base caps and earned increases

The optional ingest settings define the base daily caps before an earned increase:

```sh
TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY=200
TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY=2147483648
```

Set both variables and restart ingest. Both must be positive integers. The count must not exceed 1,000, and bytes must not exceed 5,368,709,120 (5 GiB). Partial or invalid configuration refuses startup with a fixed configuration label.

For an account on the bounded admission path, the server reads the effective allowance already validated and applied by admission. When `effective_allowance` exceeds the policy's `bounded_allowance`, it scales each base cap by their ratio:

```text
daily cap = min(hard ceiling, floor(base cap × effective_allowance / bounded_allowance))
```

Count and bytes scale independently, with integer division rounding down, and retain the hard ceilings of 1,000 uploads and 5 GiB. If both base settings are absent, earned growth uses the shared defaults of 50 uploads and 200 MiB (209,715,200 bytes) as its base. For example, a validated ratio of 2 advertises 100 uploads and 400 MiB without any operator base settings. With no configured base and no validated growth above 1, the response is `{"allowance":null}`.

Both existing external-trust and tiered-trust admission modes supply the account's applied effective allowance. Their existing validation, freshness, and generation safeguards remain authoritative; this endpoint does not introduce scoring, promotion thresholds, or an independent trust decision. An account whose trust falls back to its ordinary admission allowance loses the earned uplift on a successful refresh: the response contains the configured base, or null when no base was configured. A transient status-read failure returns an error rather than pretending that trust fell back.

Legacy and invited identities that use their independent compatibility admission path keep the configured base unchanged. They do not gain an earned multiplier from this endpoint; with no configured base, they receive null.

## Endpoint and client behavior

`GET /v1/account/upload-allowance` uses normal native bearer or account-cookie authentication and session rotation. Unbound and closed accounts are refused. Successful responses carry `Cache-Control: no-store` and either:

```json
{"allowance":{"schema_version":1,"max_uploads_per_day":200,"max_bytes_per_day":2147483648}}
```

or `{"allowance":null}` when neither a configured base nor an earned increase applies. No tenant/account identifiers, trust scores, or credentials appear in the response.

Supporting clients refresh at startup and every five minutes even when their local daily cap is spent. A server allowance can raise default limits; explicitly chosen local limits remain in force separately for count and bytes. Today's usage counters are preserved, so a higher cap can immediately release an already-approved queue. Defaults remain 50 uploads and 200 MiB when no allowance applies. Existing clients keep their prior behavior.

Clients retain the last valid allowance for the current signed-in identity after a transient refresh failure, until restart. A successful null response removes the previous uplift. Signing out or switching account, tenant, or server discards the old allowance. Allowances are runtime state, separate from saved local settings.

On upgrading legacy local settings, nondefault count/byte values are preserved as explicit choices. A legacy value equal to the default cannot be distinguished from an untouched setting and follows the server; future explicit writes retain that choice even when it equals the default.

Unsetting both operator variables and restarting removes the configured base, but accounts with validated earned growth still receive caps scaled from the shared defaults. Earned uplift follows the existing admission trust controls and falls back when admission no longer applies that growth. Supporting clients adopt the resulting base or null response at their next successful refresh; uploads already sent and daily usage counters are unchanged.
