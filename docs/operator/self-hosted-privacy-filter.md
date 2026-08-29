# Runbook: self-hosted privacy filter

Brings up `openai/privacy-filter` on the pilot host and cuts the privacy-filter
backend over from `near-ai` to `self-hosted`.

Design: [`../superpowers/specs/2026-08-29-self-hosted-privacy-filter-design.md`](../superpowers/specs/2026-08-29-self-hosted-privacy-filter-design.md)

## Why cut over

The hosted endpoint serves a model reporting `context_length: 512` behind an
internal splitter and fails above roughly 3,000 input tokens, so the `near-ai`
backend windows every field at a 2,000-token budget and issues those windows
one at a time. Locally the model has its real 128k context: one field, one
request. Unredacted contributor prose also stops leaving the host.

## Preconditions

- PR #495 merged and a Cloud Build published from that commit.
- A scheduled downtime window. **The resize stops the instance.**
- `gcloud auth list` shows an account with compute admin on
  `tracecommons-pilot-2026`.

## 1. Resize the host

`tc-pilot-host` is `e2-standard-2` (2 vCPU / 8 GB) and is already CPU-starved by
the bge-large ONNX embedder before anything is added. The filter needs roughly
3 GB resident at bf16.

Go to `e2-standard-4`, not larger: with 50M active parameters per-token compute
is small, and the binding constraint is memory shared with ingest and the
embedder.

**Cost.** E2 pricing is linear in vCPU and RAM, so this doubles the VM line
item exactly. From the Cloud Billing catalog API for `us-central1`, on-demand,
priced 2026-08-29: `E2 Instance Core` $0.021811590/vCPU-hour and
`E2 Instance Ram` $0.002923530/GiB-hour.

| Machine type | vCPU | RAM | Hourly | ~Monthly (730h) |
|---|---|---|---|---|
| `e2-standard-2` (current) | 2 | 8 GiB | $0.06701 | $48.92 |
| `e2-standard-4` (target) | 4 | 16 GiB | $0.13402 | $97.84 |

**Delta: +$48.92/month** at on-demand rates. Sustained-use discounts apply to
E2 and reduce both figures for a full month of running, so treat these as the
ceiling rather than the bill.

```sh
gcloud compute instances stop tc-pilot-host \
  --project tracecommons-pilot-2026 --zone us-central1-a
gcloud compute instances set-machine-type tc-pilot-host \
  --project tracecommons-pilot-2026 --zone us-central1-a \
  --machine-type e2-standard-4
gcloud compute instances start tc-pilot-host \
  --project tracecommons-pilot-2026 --zone us-central1-a
```

Confirm the services came back before continuing:

```sh
systemctl is-active cloud-sql-proxy trace-commons-upload-claim-issuer trace-commons-ingest
```

## 2. Install the filter service

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin tc-privacy-filter
sudo mkdir -p /opt/tracecommons-privacy-filter/models
sudo chown -R tc-privacy-filter:tc-privacy-filter /opt/tracecommons-privacy-filter

sudo -u tc-privacy-filter python3 -m venv /opt/tracecommons-privacy-filter/venv
sudo -u tc-privacy-filter /opt/tracecommons-privacy-filter/venv/bin/pip install \
  -r ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/requirements.txt

sudo install -o tc-privacy-filter -g tc-privacy-filter -m 644 \
  ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/app.py \
  /opt/tracecommons-privacy-filter/app.py
```

## 3. Stage the weights

Run before first start. The unit sets `HF_HUB_OFFLINE=1`, so if the weights are
not on disk the service fails to start rather than fetching them on a request
path.

```sh
sudo -u tc-privacy-filter \
  VENV=/opt/tracecommons-privacy-filter/venv \
  ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/stage-weights.sh
```

The script refuses to report success on an empty directory.

## 4. Start it

```sh
sudo install -m 644 \
  ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/trace-commons-privacy-filter.service \
  /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now trace-commons-privacy-filter
curl -s localhost:8471/healthz
```

### Verify the offset convention against the real model

The shim's CI tests stub the model. This is the check against real weights, and
it is the one that matters: if offsets were byte offsets rather than codepoints,
redaction would land on the wrong characters, leave the PII in place, and report
success.

```sh
curl -s localhost:8471/v1/privacy/classify \
  -H 'content-type: application/json' \
  -d '{"model":"openai/privacy-filter","input":"Ping 大三 about bob@example.com today"}'
```

The email starts at **codepoint 14** and byte 18 (the two CJK characters are 3
bytes each). A `start` of 14 is correct. A `start` of 18 means byte offsets.

You should not see 18: the shim re-slices the input by the offsets it is about
to return and fails the request with a 500 naming an "offset convention
mismatch" if they disagree with the model's own matched text. If you get that
500, **stop** — do not cut the backend over.

Confirm it is loopback-only:

```sh
ss -ltnp | grep 8471   # expect 127.0.0.1:8471, never 0.0.0.0
```

## 5. Deploy the ingest binary

The backend compiles in through the protocol crate's dependency features, so
the existing build command needs no change.

```sh
deploy/pilot-gcp/pull-and-install.sh ingest <short-sha>
```

Always pass the tag. `latest.txt` names the last build that published, so
running against an in-flight build reinstalls the previous binary and prints
"done."

Per [`pilot-host-checkout`](../../CLAUDE.md), the host checkout is not the
deployed code. Verify by string marker, not `git log`:

```sh
strings /opt/tracecommons/bin/trace-commons-ingest | grep -c self_hosted
```

## 6. Cut the backend over

In `/etc/tracecommons/ingest.env`:

```sh
TRACE_PRIVACY_FILTER_BACKEND=self-hosted
TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL=http://127.0.0.1:8471/v1
TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1
```

Keep `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1` set. A broken shim then refuses
the boot instead of silently degrading to deterministic-regex-only redaction.
Leave the `TRACE_NEAR_AI_PRIVACY_*` values in place; they cost nothing and make
rollback a one-line edit.

```sh
sudo systemctl restart trace-commons-ingest
journalctl -u trace-commons-ingest -n 50 | grep 'privacy filter backend resolved'
curl -s localhost:PORT/health | jq .privacy_filter_backend
```

Both must say `self_hosted`. Note that application logs go to
`/var/log/tracecommons/ingest.log`, not the journal — a clean `journalctl`
proves nothing on this host.

## 7. Do not drain the backlog yet

Two follow-ups gate it, and they are not in PR #495:

1. **Shadow comparison.** Run both backends over a sample and diff the spans,
   hash-only. Same weights does not imply same output when one side wraps a
   512-context model in a splitter. Record the agreement bar *before* running
   it, so the bar is not chosen after seeing the result.
2. **Attempt-counter reset.** Submissions that exhausted
   `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` against a failing backend stay
   invisible to enumeration even once it is healthy.

Count the real backlog before planning it. Figures in earlier documents are
stale.

## Rollback

```sh
# In /etc/tracecommons/ingest.env
TRACE_PRIVACY_FILTER_BACKEND=near-ai
sudo systemctl restart trace-commons-ingest
```

The `near-ai` path is untouched by this change. The filter service can be left
running or stopped independently; ingest does not consult it once the backend
is `near-ai`.
