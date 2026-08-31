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
- **At least 10 GB free on `/`.** The venv is ~2 GB (CPU torch) and the
  checkpoint 2.7 GB, but `snapshot_download` also fetches an `onnx/` tree of
  ~10.5 GB unless restricted -- it took the host to 98% before being stopped.
  Grow the disk rather than running close to the line.
- **At least 8 GB RAM free.** The loaded service holds **~6 GB resident**, not
  the ~3 GB a bf16 parameter count suggests. On the original `e2-standard-2`
  (8 GB total, shared with ingest and the bge-large embedder) it would not have
  fit at all.
- **A CPU with AMX/AVX512-BF16** (C3 or newer). See the throughput section.

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
sudo -u tc-privacy-filter env HOME=/opt/tracecommons-privacy-filter \
  /opt/tracecommons-privacy-filter/venv/bin/pip install \
  -r ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/requirements.txt

sudo install -o tc-privacy-filter -g tc-privacy-filter -m 644 \
  ~/trace-commons-server/deploy/pilot-gcp/privacy-filter/app.py \
  /opt/tracecommons-privacy-filter/app.py
```

`requirements.txt` pins `torch==2.13.0+cpu` from PyTorch's own index on Linux.
Do not "simplify" that to plain `torch`: on linux-x86_64 the default PyPI wheel
pulls the entire CUDA runtime -- several GB, `nvidia-cufft` alone is 214 MB --
onto a host with no GPU. Confirm after installing:

```sh
sudo -u tc-privacy-filter /opt/tracecommons-privacy-filter/venv/bin/python \
  -c 'import torch; print(torch.__version__, torch.cuda.is_available())'
# expect: 2.13.0+cpu False
sudo -u tc-privacy-filter /opt/tracecommons-privacy-filter/venv/bin/pip list \
  --format=freeze | grep -i nvidia   # expect: no output
```

If you ever need to stop a run-away install, kill it by a self-excluding
pattern such as `sudo pkill -f '[v]env/bin/pip'`. A plain
`pkill -f 'pip install'` also matches the SSH command line carrying that
string and will drop your own session.

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

**Point `OPF_CHECKPOINT` at the `original/` subdirectory, not the repo root.**
The HF repo ships two things: a transformers-style `config.json` at the root,
and opf's own native checkpoint under `original/` with its own `config.json`
carrying the `encoding` field (`o200k_base`) that opf's loader requires. Aimed
at the root, the service starts, reports healthy, and then fails every request
with `ValueError: Checkpoint config field encoding must be a non-empty string`.

The repo also ships an `onnx/` tree of ~10.5 GB -- fp16, quantized, q4 and q4f16
variants for transformers.js and ONNX runtimes. **None of it is used here** and
a full `snapshot_download` will pull all of it; it filled the pilot host to 98%
before being stopped. Fetch only what is needed.

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

## Throughput: measured, and it is the deciding constraint

Measured on `tc-pilot-host` against real weights, warm, 2026-08-29:

| Input | e2-standard-4 (2 thr) | e2-standard-4 (4 thr) | c3-standard-4 (AMX) |
|---|---|---|---|
| 36 chars | 1.0 s | -- | -- |
| 1,024 chars | 53.0 s | 34.5 s | 17.5 s |
| 2,048 chars | 86.3 s | -- | 34.8 s |
| 4,096 chars | -- | -- | 69.4 s |
| 8,192 chars | -- | -- | 140.2 s |
| 16,384 chars | -- | -- | 279.8 s |
| ~41,000 chars | >13 min, unfinished | -- | -- |

Linear in input length. **~58 characters/second on C3**, ~46/s on dense PII
prose. Three identical 2,048-char payloads took 35.38 s, 35.57 s, 35.73 s, so
this is steady-state compute, not `torch.compile` warm-up.

Moving from E2 to C3 bought **2x**, not the order of magnitude the instruction
sets suggest. The hosted endpoint classifies a 2,000-token window in ~4.5 s, so
self-hosting on CPU is roughly **20x slower end to end**.

Detection quality is not the problem. On 2 KB of realistic PII prose the model
returned 52 spans across all five expected categories (11 person, 11 email, 10
phone, 12 address, 8 account_number), identical across three runs. **The adapter's default timeout is 30 s**
(`TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS`), so on this hardware any field
beyond a few hundred characters fails every attempt. Cutting over would wedge
the PII backstop harder than the hosted backend does.

E2 was the worse of the two for a specific reason worth keeping: `/proc/cpuinfo`
there reports **`avx2` only**, with no AVX-512, AVX512-BF16 or AMX, while the
checkpoint is `param_dtype: bfloat16` -- so every matmul ran emulated. C3
(Xeon Platinum 8481C) has `amx_bf16`, `amx_tile`, `avx512_bf16` and
`avx512_vnni`, and torch reports both `avx512_bf16` and `amx` available. That
is worth 2x and no more; the remaining gap is opf's PyTorch MoE path, not the
silicon.

Raising `OMP_NUM_THREADS` from its default of 2 to 4 was worth 1.5x on E2. Set
it explicitly; nothing else in tuning moved the number.

### What this means for configuration

One request carries a whole field, so the 30-second adapter default is far too
low and the 16 MB `MAX_INPUT_BYTES` default is far too high -- a 16 MB field at
this rate is roughly 78 hours. Both must be set:

```sh
TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS=600000        # 10 minutes
TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES=32768    # ~9.5 min worst case
```

**Fields above the cap fail closed and stay held.** The hosted backend covers
them by windowing, so this is a real capability regression traded for keeping
prose on the host. Decide deliberately.

### If throughput matters more than locality

1. **ONNX runtime with the quantized weights.** The repo ships `model_q4.onnx`,
   `model_q4f16.onnx` and `model_fp16.onnx` for exactly this; transformers.js
   runs this model in a browser, so a fast CPU path exists. opf has no ONNX
   support, so this means owning the Viterbi decode and BIOES span extraction
   -- and re-earning the offset guarantee.
2. **A GPU host.** 50M active parameters is trivial on an L4.
3. **Stay on `near-ai`.** Rollback is one env line.

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

## Draining a backlog on an ephemeral GPU

The CPU shim runs at ~58 characters/second. An L4 runs the same model at
**~44,000** measured end-to-end through the same shim -- about 750x -- so a
backlog that takes days on CPU is one short session. Keep no GPU between
sessions.

`scripts/operator/gpu-privacy-filter-batch.sh` does the whole cycle:

```sh
scripts/operator/gpu-privacy-filter-batch.sh up      # create + provision
scripts/operator/gpu-privacy-filter-batch.sh attach  # local shim down, tunnel up
scripts/operator/gpu-privacy-filter-batch.sh status  # watch the held count
scripts/operator/gpu-privacy-filter-batch.sh detach  # tunnel down, local shim up
scripts/operator/gpu-privacy-filter-batch.sh down    # delete, and VERIFY
```

Measured on an L4, warm, through the shim:

| chars | seconds | chars/sec |
|---|---|---|
| 1,024 | 0.05 | 22,004 |
| 16,384 | 0.37 | ~44,000 |
| 262,144 | 6.11 | 42,918 |
| 1,048,576 | 24.90 | 42,103 |

The 14 MB submission that consumed 50 hours of CPU takes about 5.5 minutes.

### Why ingest needs no change

`attach` stops the pilot's local shim and opens an IAP tunnel on the same
`127.0.0.1:8471`. Ingest keeps addressing loopback, so there is no config
change, no restart, and no code change. Traffic is encrypted and authenticated
by IAP rather than crossing the VPC in plaintext -- which matters, because the
self-hosted adapter has **no TLS guard** for non-loopback endpoints.

**No database credentials, KEK access or artifact keys reach the spot VM.** It
runs the stateless classifier only; ingest still does envelope decrypt and
release.

`detach` and `down` are separate so a failed teardown never leaves ingest
without a filter. If the GPU is preempted mid-drain, ingest sees a transport
error, which the adapter types as transient and does not charge to the trace.

### Two traps, both hit while building this

- The image needs **`python3-dev`**. Without it Triton cannot JIT CUDA kernels,
  and the real cause (`Python.h: No such file or directory`) is buried under a
  `CalledProcessError` that reads like a CUDA fault.
- Stage the checkpoint with **`allow_patterns=["original/*"]`**. A full
  `snapshot_download` also pulls a ~10.5 GB `onnx/` tree nothing here uses; it
  took the pilot host to 98% disk.

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
