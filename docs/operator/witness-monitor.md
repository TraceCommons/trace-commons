# Witness overload monitor

Answers one question: **is the redaction witness overloaded right now, and is it
blocking uploads?**

The witness (a Phala dstack CVM) bounds concurrent `POST /v1/witness` requests.
When every slot is held it refuses new requests, and a request that runs past
its limit is abandoned. A contributor sees that as a preview or upload that
hangs or fails. On 2026-11-05 the live witness began refusing every new request
after about 122 s, and nothing the operator could read said why. The witness
now serves `GET /v1/diagnostics` (PR #1975), and this monitor reads it.

## What exists

| Piece | Where |
|---|---|
| Poller and deploy check | `scripts/operator/witness_monitor.py` (stdlib Python, label-only) |
| Self-tests | `scripts/operator/test_witness_monitor.py`, run by the `Run witness monitor self-tests` CI step |
| Timer and service | `deploy/pilot-gcp/systemd/trace-commons-witness-monitor.{timer,service}` |
| Env template | `deploy/pilot-gcp/witness-monitor.env.template` |

Nothing here is installed anywhere yet. The live witness has no
`/v1/diagnostics` until it is redeployed with #1975, so the monitor reports
`unreachable` (`http_4xx`) against it today, correctly.

## What it reads

Only these fields of `witness-diagnostics-v1`, under `requests`:
`max_concurrent`, `in_use`, `completed_2xx`, `completed_4xx`, `completed_5xx`,
`saturated_refusals`, `timed_out`, plus top-level `schema` and `uptime_secs`.
The counters are cumulative since the witness process started, so the monitor
works on the change between polls. The classifier outcome counts on the page are
not alerted on; read them by hand from the page when triaging.

## Alerts

| Alert | Raised when |
|---|---|
| `unreachable` | The page did not answer, answered non-2xx, or was not JSON. The record's `failure` field says which: `timeout`, `connection`, `http_4xx`, `http_5xx`, `body_not_json`. |
| `schema_mismatch` | The body is not `witness-diagnostics-v1` or lacks a field above. A witness this monitor cannot read is not a healthy one. |
| `saturated_refusals_rose` | `saturated_refusals` rose by at least `SATURATED_DELTA` since the last answered poll: the witness refused a request because every slot was held. |
| `timed_out_rose` | `timed_out` rose by at least `TIMED_OUT_DELTA`: a request ran past the witness's own limit and was abandoned. |
| `slots_full_sustained` | `in_use` equalled `max_concurrent` on `FULL_POLLS` consecutive polls. |

A first poll, or the first poll after a witness restart (uptime or a counter
went backwards, recorded as `restarted: true`), sets a new baseline and raises
no delta alert. An unreachable poll keeps the old baseline.

## Thresholds

Set in `/etc/tracecommons/witness-monitor.env`; defaults apply when unset.
A bad value refuses the run (exit 2, `WitnessMonitorRefused:config_invalid:<name>`)
rather than falling back.

| Variable (prefix `TRACE_COMMONS_WITNESS_MONITOR_`) | Default | Meaning |
|---|---|---|
| `URL` | none, required | The witness's diagnostics URL. Never printed or stored. |
| `TIMEOUT_SECS` | `10` | Per-poll HTTP timeout. The page does constant work and sits outside the slot bound, so it answers when the witness is saturated. |
| `SATURATED_DELTA` | `1` | Rise in `saturated_refusals` per poll that alerts. |
| `TIMED_OUT_DELTA` | `1` | Rise in `timed_out` per poll that alerts. |
| `FULL_POLLS` | `3` | Consecutive full-slot polls that alert (three minutes at the one-minute timer). |
| `STATE_DIR` | `/var/lib/tracecommons/witness-monitor` | Holds `records.jsonl` and `state.json`. The unit sets it. |
| `MAX_RECORDS_BYTES` | `10485760` | `records.jsonl` is renamed to `records.jsonl.1` (one generation) when it reaches this size. |

The defaults alert on any single refusal or timeout. That is deliberate for a
first pass: either one is a contributor-visible failure. Raise the deltas if it
proves noisy.

## What is stored

One JSON line per poll in `records.jsonl`: timestamp, `status` (`ok` or
`alert`), the alert labels, the failure label when unreachable, and the counts
above with their deltas. `state.json` holds the last counters. No URL, key,
measurement, signing address or exception text is written to either file, to
stdout, or to the journal (hash-only rule; tests assert it).

## Installing it (owner step, not done)

```sh
sudo install -m 0755 scripts/operator/witness_monitor.py /opt/tracecommons/bin/
sudo install -m 0640 -o root -g tracecommons deploy/pilot-gcp/witness-monitor.env.template /etc/tracecommons/witness-monitor.env   # then edit the URL
sudo install -m 0644 deploy/pilot-gcp/systemd/trace-commons-witness-monitor.{service,timer} /etc/systemd/system/
systemd-analyze verify /etc/systemd/system/trace-commons-witness-monitor.service
sudo systemctl daemon-reload
sudo systemctl enable --now trace-commons-witness-monitor.timer
systemctl list-timers trace-commons-witness-monitor.timer
```

An alert makes the unit exit 1, so it shows under `systemctl --failed` and in
`journalctl -u trace-commons-witness-monitor`. Read recent rows with
`tail -n 20 /var/lib/tracecommons/witness-monitor/records.jsonl`.

## The deploy check

`witness_monitor.py check` is stateless: one fetch, no record, no state file.
It prints one line and exits 0 or 1:

```
WitnessHealthOK: in_use=0 max_concurrent=4
WitnessHealthFailure:unreachable:http_4xx
WitnessHealthFailure:slots_full
WitnessHealthFailure:schema_mismatch
```

It is the last step of the witness deploy smoke in `deploy/witness/README.md`.
It is not a rollout-smoke check: rollout-smoke evidence is tenant-scoped and
written through the ingest admin API, and the witness has no tenant and no
operator credential by design.

## Alert delivery: not chosen

Delivery is an owner decision. What the repo already has:

- **Failed systemd units.** The only alert surface in use today
  (`systemctl --failed`). The staging README says of its backup timers "there is
  no alert yet". This monitor works with it as shipped.
- **systemd `OnFailure=`.** Not used anywhere in the repo, but a drop-in naming
  a delivery unit turns a failed monitor run into a message without changing
  the monitor.
- **Cloud Logging.** `pilot-dashboard.md` suggests it as the log aggregator for
  hash-only error classes. Nothing in the repo configures it or an alerting
  policy, notification channel or uptime check, and no mail or chat sender
  exists in the scripts.
- **Prometheus text at `/v1/admin/operational-metrics`.** Ingest only; nothing
  scrapes it from this repo, and it does not cover the witness.

Recommendation: ship with the failed-unit surface first, then add one
`OnFailure=` drop-in that posts to the channel the owner already watches. A
Cloud Monitoring policy on a log-based metric is the better long-term fit if
Cloud Logging is adopted for the pilot dashboard, but it needs cloud resources
and is not worth building before that choice.

## Troubleshooting

- **`unreachable` with `http_4xx`**: the witness build predates #1975, or the
  URL is wrong.
- **`unreachable` with `timeout` or `connection`**: the CVM is down or the
  gateway is not forwarding. The page is outside the slot bound, so a saturated
  witness still answers; a non-answer is worse than saturation.
- **`saturated_refusals_rose` or `slots_full_sustained`**: more concurrent
  witness work than `max_concurrent` allows. Look at the page's classifier
  `in_flight` and latency buckets: slow NEAR AI responses hold slots.
- **`timed_out_rose`**: requests outrun the witness's own limit; large traces
  through the classifier are the known cause
  (`witness-load-testing.md`).
- **`schema_mismatch`**: the page changed shape. Update `REQUEST_FIELDS` and the
  pinned fixture in `test_witness_monitor.py` together.
