# Witness submit-then-poll

Status: APPROVED design. Owner decisions 2026-11-05 (section 2) override the
sketch in section 5 of `2026-11-05-witness-scaling-design.md` (PR #1968)
where they differ. Date: 2026-11-05.

Code references are to `origin/main` at 8a8eda07e.

## 1. Why

A witness review is one HTTP request today: the client sends the raw
session to `POST /v1/witness` and waits, with nothing on the wire, for up to
540 s. Every timeout between client and witness is coupled to the review's
length (the dstack gateway's 10-minute idle close is the binding one), and a
dropped connection loses the work. Submit-then-poll splits the request in
two: a short submit that answers `202` at once, and short polls until the
result is ready.

## 2. Owner decisions (2026-11-05)

1. **Raw sessions are held in memory only, until processed.** Never written
   to disk, sealed or not.
2. **Redacted results go to disk, encrypted**, under a key derived inside
   the CVM from dstack's key service (`GetKey`; `/v0/GetKey` works on dstack
   0.5.9). Only a witness with the same measurement can decrypt. Each result
   is deleted on its first successful fetch, or after 24 hours. Results
   survive a witness restart. After a redeploy with a new measurement, old
   results are unreadable and the client resubmits. Disk use is bounded by
   the job count times the result size. In-progress jobs are lost on
   restart, and the client resubmits.
3. **No new user-facing copy.** The app keeps its existing privacy-check
   status text. Queue position stays internal: it feeds `poll_after_secs`
   and is never shown.
4. **Instance-addressed polling.** A job lives on one instance, the `202`
   names it, and the client polls that instance via the gateway. If the
   instance is gone, the client resubmits through the app-id URL.
5. **`POST /v1/witness` is retired in this witness version.** It is not kept
   for old clients. **Ordering is a hard constraint: the client release that
   polls must ship before, or with, the polling witness.** A client that
   only knows `POST /v1/witness` gets `404` from this witness and cannot
   witness anything.

## 3. Routes

| Route | Body / headers | Answer |
|---|---|---|
| `POST /v1/witness/jobs` | the body `POST /v1/witness` took, unchanged; `x-trace-witness-fetch-secret-sha256: <64 hex>`; optional `x-trace-witness-workload: background` | `202 {job_id, poll_after_secs, queue_position, instance_id?}`, or an immediate refusal |
| `GET /v1/witness/jobs/{job_id}` | `x-trace-witness-fetch-secret: <64 hex>` | `202 {state: "pending", poll_after_secs, queue_position}`; or the finished answer; or `404 witness_job_gone` |
| `DELETE /v1/witness/jobs/{job_id}` | `x-trace-witness-fetch-secret: <64 hex>` | `204`, or `404 witness_job_gone` |

- **The finished answer is byte-for-byte what `POST /v1/witness` answered**
  for the same body: `200` with the envelope as the body and the certificate
  and signature in `x-trace-witness-certificate` / `x-trace-witness-signature`
  (the structured shape), or the same refusal label and status
  (`503 witness_redaction_failed`, `403 witness_inference_*`,
  `504 witness_request_timed_out`, ...). The client's response handling is
  unchanged; only how the response is obtained changes.
- **The submit validates at once.** An oversized body is `413
  witness_request_too_large`, a malformed one `400 witness_request_malformed`,
  a missing or malformed secret hash `400 witness_fetch_secret_malformed`, a
  malformed workload header `400 witness_workload_malformed`. These are not
  turned into jobs.
- **Busy is the existing busy.** `503 witness_saturated` with `Retry-After`
  when the job table is full, when the background share is full for a
  background job, or when the jobs ahead would, at the recent pace,
  outlast the job's deadline (the same `ClassifyQueue::would_exceed` check
  as today). So a job is never accepted only to expire before it runs.
- **`gone` is one answer for every miss**: an unknown id, a wrong secret, an
  expired or already-fetched result, an unreadable sealed file. One label
  and one status, so the route is not an oracle for which ids exist.
- `GET` and `DELETE` sit outside the load bound, like `/v1/attestation`:
  they are cheap and must keep answering while the witness is saturated.

`/v1/witness/admission` and `/v1/witness/token-bundle` are **unchanged**
long requests in this version (decision 5 names `POST /v1/witness` only).
Moving them onto jobs is a follow-up; see section 10.

## 4. Who may fetch

`job_id` is 128 random bits (`OsRng`), hex. An id travels in a URL, so it is
not the capability. The client draws a 32-byte `fetch_secret`, sends
`sha256(fetch_secret)` with the submit, and sends the secret itself on every
`GET` and `DELETE`. The witness hashes the presented secret and compares the
32-byte digests in constant time (an XOR fold over fixed-length digests).

Neither the id nor the secret is stored raw. The job table is keyed by
`sha256(job_id)`, and the sealed file is named by that hash. No log line,
diagnostic or file name carries an id or a secret.

## 5. The nonce and the certificate (unchanged)

The client's nonce is bound exactly as today, and today it is bound
**into the attestation quote, not into the certificate**:

- `GET /v1/attestation?nonce=` lays the nonce next to the signing address
  in the quote's report data (`witness_service/enclave.rs`,
  `witness_report_data`).
- The certificate (`certificate_json`, `witness_service/http.rs`) carries
  the redacted digest, verdict, policy, measurement and timestamp, and no
  nonce. `WitnessRequestBody` (`deny_unknown_fields`) has no nonce field.
- The client verifies the certificate's signature recovers to the signing
  address the nonce-bound quote pinned (`verify_certificate`).

So the job submit carries no nonce, and no certificate field changes. The
binding is unchanged by splitting the request: whichever response carries
the certificate, it is signed by the key the client pinned, over the bytes
the client receives.

## 6. Execution and deadlines

A job is the old request, run inside the witness instead of over the wire.
The submit handler parses and validates the body, registers the job, and
spawns a task that drives the same internal `POST /v1/witness` handler
through the same `WitnessLoadBound` middleware (permits, background
reservation, 540 s deadline). That route is mounted on an internal router
only, never on the listener. Its answer becomes the job's result.

- **Arrival order.** The classify queue (#1973) is a fair mutex, so jobs
  classify in the order they were accepted, as requests did.
- **Deadline.** A job's deadline is the request timeout
  (`TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS`, 540 s), counted from
  acceptance, waiting included, as it was for a request. A job that runs
  out is finished with `504 witness_request_timed_out`, which the client
  already reads as a timeout. There is no separate pending lifetime: the
  admission check refuses a job the queue ahead would make miss this
  deadline.
- **Concurrency.** Active jobs (accepted, not finished) are capped at
  `TRACE_COMMONS_WITNESS_MAX_CONCURRENT_REQUESTS`, so the job table never
  asks the load bound for a permit it does not have. Background jobs are
  capped at that minus the reserved interactive slots.
- **Raw bytes.** The body is held as one in-memory buffer from submit until
  the task hands it to the handler; it is dropped when the handler returns.
  It is never written anywhere.
- **Cancellation.** `DELETE` of an active job aborts its task: the body is
  dropped, any classifier call in flight is dropped with it, and no result
  is produced.
- **`poll_after_secs`.** `queue_position` is the number of active jobs
  accepted before this one. `poll_after_secs = clamp(ceil((position + 1) *
  mean_turn / 2), 2, 30)`, or 5 when no turn has been timed yet.

## 7. Sealed results on disk

### 7.1 The key

`results_key = HKDF-SHA256(ikm = GetKey(path = "trace-commons/witness-results/v1"),
salt = "tcwitness-results-v1", info = measurement label)`.

- The `GetKey` path differs from the signing key's `vms`; only the path
  separates dstack-derived keys (`enclave.rs` module docs).
- `GetKey` alone derives from the **app id**, not the measurement (scaling
  spec 3.1): every image under one app id would get the same bytes. Mixing
  in the boot measurement label (`mrtd:..+mrconfigid:..`, read from a signed
  quote at boot) is what gives decision 2's property: a redeploy with a new
  measurement derives a different key and cannot open old results. This is
  the honest-image property. An image that lies about its own measurement
  could derive the old key; what stops such an image is the app's KMS
  policy over compose hashes, the same thing that guards the signing key.
- Replicas of one app at one compose hash share the key, which is harmless:
  a job is fetched only from the instance that holds its file.

### 7.2 The file

One file per finished job, `<results dir>/<hex sha256(job_id)>.sealed`,
written to a temporary name and renamed. Contents: a version byte, a
12-byte random nonce, and AES-256-GCM ciphertext with the id hash as
associated data, so a file renamed onto another id fails to open. The
plaintext carries the secret hash, the expiry (Unix seconds), the status,
the response headers that matter (content type, certificate, signature,
`Retry-After`) and the body.

### 7.3 Lifetime and bounds

- Deleted when it is fetched (the table entry is removed before the file is
  read, so two concurrent fetches cannot both get it), when it is
  cancelled, or 24 hours after it finished (a sweep every minute).
- At startup the directory is swept: a file that does not open (another
  measurement, corruption) or has expired is deleted; the rest are indexed
  back into the table as finished. Temporary files are deleted.
- Finished results are capped at `TRACE_COMMONS_WITNESS_MAX_STORED_RESULTS`
  (default 256), each at `TRACE_COMMONS_WITNESS_MAX_RESULT_BYTES` (default
  4 MiB, four times the 1 MiB request cap). Worst case on disk is 1 GiB. The
  CVM's disk size is not recorded in `deploy/witness/README.md`; Phala
  deploys default to a larger data disk than this, and the operator should
  confirm it before raising either value. A full store answers busy at
  submit, before any work is done.
- A result over the size cap is finished as `503 witness_redaction_failed`
  and nothing is written.

### 7.4 Fail closed

- No usable key (agent refused, malformed answer) or an unwritable results
  directory: **the witness does not start.**
- A seal or write failure: the job's result is dropped and the job reads as
  `gone`; the client resubmits. Nothing is ever served or kept in plaintext.
- An open (decrypt) failure: the file is deleted and the job reads as
  `gone`.

### 7.5 Deploy

The compose is `read_only: true` with a 16 MiB `/tmp`. Results need a named
volume (`witness-results` at `/var/lib/trace-commons-witness/results`),
which lands on the CVM's encrypted data disk. Like every compose change it
moves MRCONFIGID, so **this release is a measurement rollover** (README
7.2 procedure: publish old + new, move, publish new only), as well as a
client-first rollout.

## 8. Instance-addressed polling

- The witness reads its instance id from the guest agent's `Info`
  (`/v0/Info`, falling back to `/Info` on 404, like `GetKey`). dstack
  serializes the protobuf `bytes instance_id` as hex. If `Info` fails, the
  witness starts anyway and omits `instance_id` from the `202`; that is
  logged as `instance_addressed=false`. A single-instance deployment polls
  correctly either way.
- The dstack gateway routes the first label of the host, `<id>[-<port>[s|g]]`,
  to an instance when `id` is an instance id and to any instance of an app
  when it is an app id (`gateway/src/proxy.rs` `parse_dst_info`,
  `main_service.rs` `select_top_n_hosts`, dstack v0.5.9).
- The client derives the poll host from the **verified witness URL**: when
  the first label's id is lowercase hex (an app id), it replaces only that
  id with the instance id and keeps the port suffix and the gateway domain.
  The instance id is accepted only as 8 to 64 lowercase hex characters.
  Anything else from the `202` cannot influence where the client connects.
  Any other URL shape is polled as is.
- The derived host is not on the config allowlist (exact match by design).
  It is permitted for `GET` and `DELETE` only, carrying the fetch secret and
  never raw content, and only because it is the allowlisted host with one
  label's id swapped for a validated hex id. The result is verified against
  the pinned signing address as before; every replica shares that key.

## 9. Client

`HttpWitnessTransport::witness` (non-admission profile) submits, then polls,
inside the transport, so `witness_contribution`'s checks are untouched.

- A fresh 32-byte fetch secret per submit, from the OS RNG (as
  `WitnessNonce::fresh`). Id and secret live in memory for the life of the
  call and are never logged.
- Polls wait `poll_after_secs` (clamped to 1..=60) with +/-20% jitter.
- `gone`, or a poll that cannot reach the instance (connect failure, or a
  response that is not a witness answer), resubmits **once** through the
  app-id URL. A second miss returns `witness_unreachable` (or
  `witness_timeout`), which the daemon already treats as transient.
- Busy at submit is `WitnessSaturated` with its `Retry-After`, as today.
- The whole review keeps the existing `WITNESS_REVIEW_TIMEOUT` (570 s)
  deadline, so the preview's 11-minute bound still holds. When it runs out,
  the client sends a best-effort `DELETE` and returns `witness_timeout`.
- No status type, FFI field or UI string changes (decision 3).

## 10. Diagnostics and logs

- The job table exposes label-only counts: `pending` (waiting for the
  classify turn), `running`, `done_unfetched`, and `expired` (results
  deleted unfetched since start). `/v1/diagnostics` (PR #1975) gains them
  when that lands; until then they are a method on the table.
- Logs are label-only: `job accepted`, `job finished` with the status
  class, `job result sealed`, `job expired`, sweep counts. Never an id, a
  secret, a body length or a hash prefix of either.

## 11. Not covered by the owner, decided here

1. The job submit carries no nonce, because no request carries one today
   (section 5).
2. The pending deadline is the existing 540 s request timeout, counted from
   acceptance; no new pending lifetime.
3. The results key mixes the measurement label into a `GetKey`-derived
   secret, because `GetKey` alone is per app id.
4. Admission and token-bundle routes stay long requests in this version.
5. Store caps: 256 results, 4 MiB each.
6. A fetch is "successful" once the witness has taken the result out of the
   table to send it; a connection that drops mid-body loses it and the
   client resubmits.
7. One in-transport resubmit on `gone` or an unreachable instance, then the
   existing transient-retry path.
8. Missing instance id is not fatal at boot.
