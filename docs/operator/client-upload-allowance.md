# Client daily upload allowance

The server can advertise larger daily upload-count and byte limits to signed-in clients. This is independent of server admission quotas; it does not approve sessions or change consent, pause, or server acceptance.

Set both variables on ingest, then restart:

```sh
TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY=200
TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY=2147483648
```

Both must be positive integers. The count must not exceed 1,000, and bytes must not exceed 5,368,709,120 (5 GiB). Partial or invalid configuration refuses startup with a fixed configuration label. Leaving both unset advertises no override. This initial policy applies equally to authenticated bound and legacy accounts.

`GET /v1/account/upload-allowance` uses the normal native bearer or account cookie authentication and session rotation. Unbound and closed accounts are refused. Successful responses carry `Cache-Control: no-store` and either:

```json
{"allowance":{"schema_version":1,"max_uploads_per_day":200,"max_bytes_per_day":2147483648}}
```

or `{"allowance":null}` when unconfigured. No tenant/account identifiers or credentials appear in the response.

Supporting clients refresh at startup and every five minutes even when their local daily cap is spent. A server allowance can raise default limits; explicitly chosen local limits remain in force separately for count and bytes. Today's usage counters are preserved, so a higher cap can immediately release an already-approved queue. Defaults remain 50 uploads and 200 MiB when no allowance applies. Existing clients keep their prior behavior.

Clients retain the last valid allowance for the current signed-in identity after a transient refresh failure, until restart. A successful null response removes it. Signing out or switching account, tenant, or server discards the old allowance. Allowances are runtime state, separate from saved local settings.

On upgrading legacy local settings, nondefault count/byte values are preserved as explicit choices. A legacy value equal to the default cannot be distinguished from an untouched setting and follows the server; future explicit writes retain that choice even when it equals the default.

To roll back, unset both variables and restart. Supporting clients revert to their local/default limits at their next successful refresh; uploads already sent and daily usage counters are unchanged.
