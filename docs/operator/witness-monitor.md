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
| `EXPORT_METRICS` | `0` | `1` writes the Cloud Monitoring gauge each poll (see "Cloud Monitoring email alerts"). Off by default so a laptop or CI run makes no GCP calls. |
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

## Cloud Monitoring email alerts

Decision (owner): alerts go out as email through Cloud Monitoring. Two
independent signals share one email notification channel:

1. **The gauge.** With `TRACE_COMMONS_WITNESS_MONITOR_EXPORT_METRICS=1`, each
   poll writes custom metrics as the host's service account, using only the
   metadata server's token and the Monitoring REST API (no library, no key
   file, no secret on the host):
   - `custom.googleapis.com/trace_commons/witness_monitor/alert`, one series per
     alert label (label `alert` = `unreachable`, `schema_mismatch`,
     `saturated_refusals_rose`, `timed_out_rose`, `slots_full_sustained`), value
     0 or 1;
   - `custom.googleapis.com/trace_commons/witness_monitor/overall_alert`, 1 when
     any alert is raised.

   The resource is `global` with the project id only. If the metadata server or
   the API is unreachable or refuses (`metadata_unreachable`,
   `metadata_bad_response`, `api_unreachable`, `api_denied`, `api_http_4xx`,
   `api_http_5xx`), the poll prints `metrics_export_failed:<label>`, records it
   as `export`, and exits 1: the unit fails and `systemctl --failed` shows it.
   Export failures never hide an alert, and an alert never hides an export
   failure.
2. **The uptime check.** Google probes `/v1/diagnostics` from outside, so a
   dead host, a dead monitor or a revoked IAM grant cannot silence the witness
   alarm. It sees only reachable or not, which is why the gauge exists.

The gauge policy also has an absence condition: if the monitor stops reporting
for 5 minutes, that opens an incident too. A silent monitor is not a healthy
one.

**Nothing below has been run.** Every command is for the owner. Each carries
`--project tracecommons-pilot-2026 --account zaki@iqlusion.io`. The recipient is
the placeholder `<ALERT_EMAIL>`; do not put an address in the repo.

The service account is `tc-pilot-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com`,
from "Prereqs to provision per pilot" and the `TC_RUNTIME_SA` export in
[`pilot-gcp-deployment.md`](./pilot-gcp-deployment.md). That document names the
account for a pilot provisioned from it; step 0 confirms that it is the one
attached to `tc-pilot-host`.

Order matters: the policies reference the custom metric, and Cloud Monitoring
creates a custom metric on its first write, so the monitor must write once
before step 5.

```sh
P=tracecommons-pilot-2026
A=zaki@iqlusion.io
SA=tc-pilot-runtime@${P}.iam.gserviceaccount.com

# 0. Read-only: confirm the host's service account and that its scopes allow
#    Monitoring writes (cloud-platform or monitoring/monitoring.write).
gcloud compute instances list --project "$P" --account "$A" \
  --filter='name=tc-pilot-host' \
  --format='value(serviceAccounts[].email,serviceAccounts[].scopes)'
# If the email differs from $SA, set SA to what it printed. If the scopes lack
# monitoring.write and cloud-platform, the VM must be stopped to change them;
# stop here and decide.

# 1. Enable the Monitoring API (a no-op if already on).
gcloud services enable monitoring.googleapis.com --project "$P" --account "$A"

# 2. The minimal grant: write metrics, nothing else.
gcloud projects add-iam-policy-binding "$P" --account "$A" \
  --member="serviceAccount:${SA}" --role=roles/monitoring.metricWriter

# 3. The email channel. Note the printed channel name,
#    projects/<project>/notificationChannels/<id>. Google emails the address a
#    verification link; the channel delivers only after it is verified.
gcloud beta monitoring channels create --project "$P" --account "$A" \
  --display-name="Witness alerts (email)" --type=email \
  --channel-labels=email_address=<ALERT_EMAIL>
CHANNEL=projects/${P}/notificationChannels/<ID from the line above>

# 4. On the host: install the monitor (see "Installing it"), set
#    TRACE_COMMONS_WITNESS_MONITOR_EXPORT_METRICS=1 in the env file, then run it
#    once by hand so the metric exists, and expect exit status 0:
sudo systemctl start trace-commons-witness-monitor.service
systemctl status trace-commons-witness-monitor.service
# An `api_denied` here means step 2 has not taken effect or the VM scopes are
# too narrow (step 0). Wait a minute after step 2 and retry once.

# 5. The gauge policy, with email when an incident opens and when it closes.
cat > witness-gauge-policy.json <<'EOF'
{
  "displayName": "Witness overloaded, unreachable or unmonitored",
  "combiner": "OR",
  "conditions": [
    {
      "displayName": "overall_alert is 1",
      "conditionThreshold": {
        "filter": "metric.type=\"custom.googleapis.com/trace_commons/witness_monitor/overall_alert\" AND resource.type=\"global\"",
        "comparison": "COMPARISON_GT",
        "thresholdValue": 0,
        "duration": "0s",
        "aggregations": [{"alignmentPeriod": "60s", "perSeriesAligner": "ALIGN_MAX"}],
        "trigger": {"count": 1}
      }
    },
    {
      "displayName": "monitor stopped reporting",
      "conditionAbsent": {
        "filter": "metric.type=\"custom.googleapis.com/trace_commons/witness_monitor/overall_alert\" AND resource.type=\"global\"",
        "duration": "300s",
        "aggregations": [{"alignmentPeriod": "60s", "perSeriesAligner": "ALIGN_MAX"}]
      }
    }
  ],
  "alertStrategy": {
    "notificationPrompts": ["OPENED", "CLOSED"],
    "autoClose": "1800s"
  },
  "documentation": {
    "content": "Witness overload monitor. Runbook: docs/operator/witness-monitor.md. The alert label is on the alert metric series; the host's records.jsonl has the counts.",
    "mimeType": "text/markdown"
  }
}
EOF
gcloud monitoring policies create --project "$P" --account "$A" \
  --policy-from-file=witness-gauge-policy.json \
  --notification-channels="$CHANNEL"

# 6. The uptime check on the witness's diagnostics page. <WITNESS_HOST> is the
#    witness's public hostname (no scheme, no path). Defaults: every minute,
#    2xx passes, probes from all regions.
gcloud monitoring uptime create "Witness diagnostics" \
  --project "$P" --account "$A" \
  --resource-type=uptime-url \
  --resource-labels=host=<WITNESS_HOST>,project_id="$P" \
  --protocol=https --port=443 --path=/v1/diagnostics --status-classes=2xx
CHECK_ID=$(gcloud monitoring uptime list-configs --project "$P" --account "$A" \
  --filter='displayName="Witness diagnostics"' --format='value(name.basename())')

# 7. The uptime policy on the same channel. It opens when the check fails from
#    more than one region for a minute, so one regional blip does not email.
cat > witness-uptime-policy.json <<EOF
{
  "displayName": "Witness diagnostics page not answering",
  "combiner": "OR",
  "conditions": [
    {
      "displayName": "uptime check failing",
      "conditionThreshold": {
        "filter": "metric.type=\"monitoring.googleapis.com/uptime_check/check_passed\" AND metric.label.check_id=\"${CHECK_ID}\" AND resource.type=\"uptime_url\"",
        "comparison": "COMPARISON_GT",
        "thresholdValue": 1,
        "duration": "60s",
        "aggregations": [
          {
            "alignmentPeriod": "1200s",
            "perSeriesAligner": "ALIGN_NEXT_OLDER",
            "crossSeriesReducer": "REDUCE_COUNT_FALSE",
            "groupByFields": ["resource.label.*"]
          }
        ],
        "trigger": {"count": 1}
      }
    }
  ],
  "alertStrategy": {"notificationPrompts": ["OPENED", "CLOSED"]}
}
EOF
gcloud monitoring policies create --project "$P" --account "$A" \
  --policy-from-file=witness-uptime-policy.json \
  --notification-channels="$CHANNEL"

# 8. Confirm what exists (read-only).
gcloud monitoring policies list --project "$P" --account "$A" \
  --format='value(displayName,enabled)'
gcloud beta monitoring channels list --project "$P" --account "$A" \
  --format='value(displayName,type,verificationStatus)'
```

The 1200 s alignment in step 7 is Google's standard form for uptime-check
policies and is why that email can lag the failure by a few minutes; the gauge
policy is the fast one.

### Test alert (run once, after step 8)

Cloud Monitoring has no "send a test" for a policy, so make the monitor raise a
real `unreachable` against an address nothing listens on, then clear it. This
exercises the whole path: token, API write, policy, channel, close email.

```sh
# Raise: one poll against a dead local port. Expect exit 1 and
# "WitnessMonitor alert unreachable". Give it two polls (about two minutes) to
# open an incident; the "opened" email arrives within a few minutes.
sudo systemd-run --wait --collect --pipe -p User=tracecommons \
  -p EnvironmentFile=/etc/tracecommons/witness-monitor.env \
  -p Environment=TRACE_COMMONS_WITNESS_MONITOR_URL=http://127.0.0.1:9/ \
  -p Environment=TRACE_COMMONS_WITNESS_MONITOR_STATE_DIR=/var/tmp/witness-monitor-test \
  /usr/bin/python3 /opt/tracecommons/bin/witness_monitor.py poll
# Repeat the command once after a minute, then let the real timer run: the next
# real poll writes 0 and the incident closes, which sends the "closed" email.
```

The test run uses its own state directory so it cannot disturb the real
baseline. Remove `/var/tmp/witness-monitor-test` afterwards.

Tradeoffs to know: the gauge is written every minute whether or not anything
changed (one series per alert label, six series in all, well inside the free
custom-metric allowance); and the email is sent by Google, so its subject and
sender are Google's, with the policy documentation text in the body. The
recipient address lives only in the notification channel, not in this repo.

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
